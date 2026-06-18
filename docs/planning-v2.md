# PR 리뷰 순서 정렬 및 클러스터링 기획안 (v2)

> v1 대비 핵심 변경
> 1. **정렬·클러스터링의 기본 엔진을 AI로 전환.** 호출 그래프 기반 DFS는 fallback으로 강등.
> 2. **"diff만 보지 않는다" 원칙을 AI 입력 정제의 근거로 재배치.** AST·심볼·관계 정보는 정렬을 *결정*하기 위해서가 아니라, AI에게 줄 입력을 *압축·정제*하기 위해 사용한다.
> 3. **레이턴시·캐싱·출력 검증을 실무 설계로 추가.** AI 호출이 들어가는 순간 결정성·비용·대기 시간이 새로운 1순위 문제가 된다.

---

## 1. 목표

이 앱은 PR diff를 단순히 파일명순, 변경량순, 디렉터리순으로 보여주는 것이 아니라, 리뷰어가 변경사항을 실제 실행 흐름대로 이해할 수 있게 정렬한다.

리뷰어가 이상적으로 따라가는 흐름은 다음과 같다.

```
entrypoint
  ↓
controller / handler
  ↓
usecase / service
  ↓
domain logic
  ↓
repository / external client
  ↓
test
```

리뷰어는 하나의 변경 흐름을 top-down으로 따라가고, 호출 흐름은 끝까지 내려간 뒤 다시 올라와 다음 흐름을 본다.

다만 이 "정확한 실행 흐름"을 정적 분석으로 안정적으로 재현하는 것은 비용 대비 회수가 나오지 않는다(→ 7장). 따라서 이 앱은 **정확함보다 "리뷰어가 보기에 그럴듯한 순서"를 우선**하며, 그 판단을 AI에 맡긴다.

---

## 2. 핵심 원칙

### 2.1 diff만 보고 정렬하지 않는다 (가장 중요)

**diff는 "어디가 바뀌었는지"를 찾는 용도로만 사용한다.** 정렬·클러스터링 판단을 diff 텍스트만으로 하지 않는다. diff hunk에는 함수 선언부, 클래스명, 타입 정보, 호출 관계가 보이지 않을 수 있기 때문이다.

따라서 앱은 다음 정보를 함께 본다.

- diff hunk
- 변경된 파일 전체 AST
- base / head 양쪽 코드
- 함수 / 클래스 symbol 정보
- 심볼 참조 관계 (import / reference)
- type usage
- import / export 관계
- test 관계

구조를 한 줄로 정리하면:

```
diff              = 변경 위치 탐지
full file AST     = 변경 위치의 의미 파악
reference graph   = 심볼 간 연결 파악
```

> **v1과의 차이 (중요)**
> v1에서는 이 정보들을 "DFS sorter의 입력"으로 썼다. v2에서는 정렬을 *결정*하는 데 쓰지 않는다.
> 대신 이 정보들은 **AI에게 줄 입력을 정제하기 위해** 쓴다. raw diff를 통째로 AI에 던지면 컨텍스트 한도·비용·환각이 모두 터진다. AST·심볼·관계 정보로 변경을 압축한 "cluster card / change card"를 만들어 AI에 전달하는 것이 이 앱의 정확도와 비용을 동시에 지키는 핵심이다.

### 2.2 리뷰 단위는 파일이 아니라 symbol

파일 전체를 하나의 리뷰 단위로 보지 않는다. 가능하면 다음 symbol 단위로 잡는다.

`function`, `method`, `class`, `type`, `interface`, `enum`, `DTO`, `entity`, `repository method`, `event payload`, `test case`, `migration`, `config block`

예: `OrderService.ts` 전체가 아니라

```
OrderService.createOrder()
OrderService.validateOrder()
OrderService.calculatePrice()
OrderService.saveOrder()
```

처럼 변경된 함수/메서드 중심으로 본다.

### 2.3 정렬·클러스터링의 source of truth는 AI, 안정성의 보루는 알고리즘

- **기본 경로:** AI가 클러스터링과 순서를 결정한다.
- **fallback 경로:** AI 실패·초과·검증 실패 시 알고리즘이 "엉성하지만 항상 동작하는" 순서로 떨어진다(→ 9장).
- AI 출력은 그대로 믿지 않고 **반드시 검증한다**(→ 8.3).

---

## 3. hunk → symbol 매핑

diff hunk가 있으면 해당 hunk가 어떤 symbol 안에 들어가는지 찾는다.

예: `src/order/OrderService.ts` line 124-131 변경

```
File:   OrderService.ts
Class:  OrderService
Method: calculatePrice()
Symbol: OrderService.calculatePrice()
```

정렬·클러스터링 대상은 line range가 아니라 symbol이다.

UI에서는 해당 symbol의 signature와 context를 함께 보여준다.

```
OrderService.calculatePrice(items: OrderItem[]): Money
File: src/order/OrderService.ts
Class: OrderService
Changed lines: 124-131
Called by:
  - OrderService.createOrder()
Calls:
  - Money.sum()
  - CouponPolicy.apply()
```

그 아래에 실제 diff를 보여준다.

### 3.1 symbol에 매핑되지 않는 hunk의 fallback (필수)

모든 hunk가 symbol에 깨끗하게 매핑되지는 않는다. 다음 케이스는 매핑이 깨진다.

- 파일 상단 import만 변경
- 클래스 바깥 최상위 상수 / 모듈 레벨 코드
- 데코레이터 / 매크로 / 어노테이션
- 파서가 지원하지 않는 언어 / 문법

이때 그 변경이 **어느 클러스터에도 안 들어가고 사라진 것처럼 보이면 신뢰를 잃는다.** 따라서 매핑 실패 hunk는 명시적으로 `Unclustered changes` 버킷에 모아 항상 노출한다. "변경이 전부 보인다"는 것은 정렬 품질과 별개로 반드시 지켜야 할 신뢰 조건이다.

---

## 4. AI 기반 클러스터링·정렬 (기본 경로)

### 4.1 단일 호출로 합치지 않는다

클러스터링과 정렬을 한 프롬프트에 몰아넣으면 출력 JSON이 커지고, 하나가 틀리면 전체가 흔들리며, 재시도 비용이 커진다. 따라서 단계를 분리한다.

```
[AI 1] 클러스터링      : 변경 symbol들을 의미 단위로 묶기
   ↓
[AI 2] 정렬            : 클러스터 내부 순서 + 클러스터 간 순서
   ↓
[AI 3] title / summary : 클러스터 라벨링·요약
```

각 단계의 출력이 작아서 검증·재요청이 쉽다.

> **레이턴시 분기 (실무 최적화)**
> 위 3단계 분리는 큰 PR 기준이다. **작은 PR은 [AI 1]+[AI 2]를 1호출로 합쳐** 레이턴시를 줄인다(변경 규모 기준 분기). title/summary는 정렬과 달리 합쳐도 안전하므로, 클러스터별 N호출이 아니라 **1호출 배치**로 처리한다.

### 4.2 클러스터의 의미

클러스터는 같은 파일이거나 그래프상 연결된 묶음이 아니라, **"리뷰어가 하나의 동작 변경으로 이해할 수 있는 변경사항 묶음"**이다.

예: 주문 생성 API 변경 / 쿠폰 적용 flow 변경 / 결제 실패 이벤트 처리 변경 / 공통 Money 계산 정책 변경 / 회원 탈퇴 배치 변경

### 4.3 클러스터 타입 (AI가 분류, 알고리즘이 힌트 제공)

알고리즘은 아래 타입에 대한 **힌트**(entrypoint 후보, 계약 변경 후보 등)를 cluster card에 담아 주고, 최종 분류는 AI가 한다.

- **Flow Cluster** — 하나의 사용자/시스템 동작 흐름 (entrypoint → controller → usecase → domain → repo → test)
- **Contract Cluster** — API schema / DTO / event payload / migration / config 계약 변경
- **Domain Concept Cluster** — 새 도메인 개념·정책 도입 (CouponPolicy, Money 정책 등)
- **Shared Foundation Cluster** — 여러 flow가 공유하는 기반 로직 (Money.round() 등)
- **Infra / Config Cluster** — 설정·빌드·DI·feature flag·환경변수 변경

### 4.4 클러스터링 관계 신호 (AI 입력 정제용)

AI가 잘 묶도록, 알고리즘이 아래 관계를 미리 계산해 cluster card에 강/중/약으로 표시해 준다. **이 신호는 AI의 판단 재료이지 자동 클러스터링 알고리즘이 아니다.** 처음부터 community detection 같은 정교한 그래프 클러스터링으로 가지 않는다(튜닝 지옥).

**강한 관계 (같은 클러스터 가능성 높음)**
- 직접 참조 관계, signature 타입 관계, 생성 관계
- Request → Command → Domain → Entity → Response 변환 관계
- Repository ↔ Entity ↔ Migration 관계
- Event publish ↔ Event payload 관계
- Test → Implementation 관계
- 같은 class 내부 public/private helper 관계

**중간 관계 (보조 신호)**
- 같은 파일 / 패키지 / feature module / domain prefix / route prefix / DB table / event topic / feature flag

**약한 관계 (거의 근거로 쓰지 않음)**
- import-only, common util 사용, logger 사용, error wrapper, base class 공유, formatting-only 변경, generated code

**hub/공통 노드 제외 룰 (품질의 80%를 결정)**
`Logger`, `DateUtils`, `StringUtils`, `JsonUtils`, `ErrorCode`, `BaseResponse`, `CommonException` 등은 여러 흐름을 억지로 하나로 붙이므로 클러스터 병합 신호로 거의 쓰지 않는다. 여러 독립 entrypoint가 공유하는 변경은 특정 flow에 욱여넣지 않고 Shared Foundation Cluster로 분리한다.

### 4.5 클러스터 내부 정렬

클러스터 내부 순서도 AI가 결정한다. 지향하는 형태는 caller → callee 방향이며, 한 함수 안에서 여러 변경 함수가 호출되면 **코드에 등장하는 순서**를 따른다. 이것이 사람이 코드를 읽는 흐름과 가장 가깝다.

```
function createOrder() {
  validateOrder();
  calculatePrice();
  saveOrder();
}
```

→ `createOrder()` → `validateOrder()` → `calculatePrice()` → `saveOrder()`

---

## 5. 새 클래스 / 자료구조 표시: Just-in-time Definition Injection

새 클래스·DTO·자료구조는 **"그걸 모르면 다음 symbol을 이해하기 어려워지는 첫 순간"**에 보여준다. 이 규칙은 정렬이 AI로 바뀌어도 그대로 유지하며, AI 정렬 결과 위에 후처리로 삽입한다.

- **signature에 등장하면 함수보다 먼저** — Request/Response DTO, Command, Query, Event payload, API schema는 해당 함수 앞에 정의를 먼저 노출.
- **함수 내부에서 처음 생성되면 생성 직전에** — `OrderDraft.from()` 보기 전에 `OrderDraft` 개요(fields / constructor / invariant / public methods)를 먼저.
- **클래스 메서드 진입 전 class overview** — `OrderPolicy.validate()` 전에 역할 / 상태 보유 여부 / constructor dependencies / public methods / 이번 PR 변경 methods 개요를 먼저. 전체 diff를 먼저 다 보여주는 게 아니라 흐름 이해에 필요한 개요만.

---

## 6. AI 입출력 계약

### 6.1 AI 입력: cluster card / change card (raw diff 아님)

raw diff를 그대로 주지 않는다. 2.1에서 모은 정보로 정제한 카드를 준다.

```json
{
  "clusterId": "cluster-1",
  "algorithmicTypeHint": "flow",
  "entrypointCandidates": ["POST /orders", "OrderController.create()"],
  "changedSymbols": [
    { "name": "CreateOrderRequest", "kind": "dto", "changeType": "modified", "summary": "couponId field added" },
    { "name": "OrderController.create()", "kind": "method", "changeType": "modified", "summary": "passes couponId to command" },
    { "name": "CreateOrderUseCase.execute()", "kind": "method", "changeType": "modified", "summary": "applies coupon policy before saving order" },
    { "name": "CouponPolicy.apply()", "kind": "method", "changeType": "added", "summary": "validates coupon and calculates discount" },
    { "name": "OrderRepository.save()", "kind": "method", "changeType": "modified", "summary": "persists couponId and discounted price" }
  ],
  "relationHints": {
    "strong": [["OrderController.create()", "CreateOrderUseCase.execute()"]],
    "weak": [["CreateOrderUseCase.execute()", "Logger.info()"]]
  },
  "contractsChanged": ["CreateOrderRequest.couponId", "OrderResponse.couponId", "orders.coupon_id migration"],
  "relatedTests": ["CreateOrderUseCaseTest", "OrderControllerTest"]
}
```

### 6.2 AI 출력: 구조화된 JSON

```json
{
  "clusters": [
    {
      "clusterId": "cluster-1",
      "title": "주문 생성 시 쿠폰 할인 적용",
      "summary": "주문 생성 요청에 couponId를 추가하고, CouponPolicy로 할인 금액을 계산한 뒤 주문 저장 시 쿠폰 정보와 할인 금액을 함께 저장합니다.",
      "orderedSymbols": [
        "CreateOrderRequest",
        "OrderController.create()",
        "CreateOrderUseCase.execute()",
        "CouponPolicy.apply()",
        "OrderRepository.save()"
      ]
    }
  ],
  "clusterOrder": ["cluster-1", "cluster-2"],
  "mergeSuggestions": [],
  "splitSuggestions": []
}
```

좋은 클러스터 이름 형식: **[대상] + [변경 동작]** (예: "회원 탈퇴 시 보유 쿠폰 만료 처리", "결제 실패 이벤트 재시도 정책 변경"). 요약은 1~3문장으로 제한한다.

### 6.3 merge / split 제안

AI는 클러스터를 직접 바꾸기보다 제안만 한다. MVP에서는 자동 적용보다 **"제안 표시"**가 안전하다(confidence 낮으면 제안만, 사용자가 직접 적용).

---

## 7. 왜 정적 호출 그래프를 기본에서 내렸나 (설계 근거)

정적 분석으로 신뢰할 만한 call graph를 만드는 것은 MVP의 진짜 난이도다. tree-sitter로 AST는 쉽게 뽑지만, 정확한 `caller → callee`는 차원이 다르다. DI, 인터페이스/동적 디스패치(`policy.apply()`의 구현체가 런타임 결정), 데코레이터, 리플렉션, 이벤트 발행/구독 분리에서 그래프가 끊긴다.

문제는 **우리가 묶고 싶은 지점이 정확히 그 끊기는 지점들**이라는 것이다 — Event publish ↔ payload, Repository ↔ Entity, interface ↔ 구현체. 즉 "변경 없는 중간 함수는 bridge로 표시"가 예외가 아니라 기본 케이스가 된다.

리뷰 순서의 가치는 "정확함"보다 "그럴듯함"에 있으므로, 이 판단은 LLM이 더 잘 맞고 투자 대비 회수도 낫다. 따라서 정적 그래프는 기본 엔진에서 내리고 fallback으로만 둔다.

---

## 8. 결정성 · 캐싱 · 검증 (AI 채택의 필수 비용)

### 8.1 결정성 문제

AI 정렬은 같은 PR을 두 번 열면 순서가 달라질 수 있다. 리뷰어가 "아까 그 변경 어디 갔지?" 하는 순간 신뢰가 무너진다.

### 8.2 캐싱 (옵션 아님, 필수)

- 결과를 **PR + commit(head) SHA 기준으로 캐싱.** 같은 head면 같은 순서 보장.
- diff가 바뀌지 않은 클러스터는 재호출하지 않는다.
- "한 번"의 단위는 PR이 아니라 **commit**이다. 리뷰 중 push가 들어오면 head SHA가 바뀌고 그 클러스터만 재분석한다. 즉 "PR당 한 번"이 아니라 "리뷰 한 건당 여러 번"이 현실이며, 캐싱이 이를 "한 번"에 가깝게 만든다.

### 8.3 출력 검증 (화이트리스트)

"지어내지 마라"는 프롬프트만으로 막지 못한다. cluster card에 넣어준 symbol 목록을 화이트리스트로 두고, AI 출력(title/summary/orderedSymbols)이 입력에 없는 symbol을 언급하면 그 출력을 버리거나 재요청한다. 프롬프트 규칙은 보조일 뿐이다.

### 8.4 레이턴시 체감 관리

순수 계산상 큰 PR 첫 분석은 수십 초~1분, 캐시 히트 시 즉시다. 이 대기를 리뷰어가 직접 기다리지 않게 한다.

- **PR open / push 이벤트 때 백그라운드 선분석** → 리뷰어가 열 땐 이미 캐시에 있음 (체감 0초). 1순위 전략.
- 선분석이 어려우면 **단계별 스트리밍** — 클러스터링 끝나면 묶음 먼저 보여주고 title/summary는 나중에 채움.
- 추가 카드: title/summary 1호출 배치(8.3 검증으로 커버), 작은 PR은 클러스터링+정렬 1호출, 정렬은 빠른 모델 / 요약만 좋은 모델로 티어 분리.

> 비용과 레이턴시는 한 몸이다. 호출 수와 출력 토큰을 줄이면 둘 다 좋아진다.

---

## 9. Fallback 경로

### 9.1 트리거 (명시)

다음 경우 fallback으로 떨어진다.

- AI 호출 실패 / 타임아웃 / JSON 파싱 실패
- 변경 규모가 커서 AI 입력 한도 초과
- AI 출력 검증 실패 (8.3) 후 재요청도 실패

### 9.2 fallback 정렬은 "싼 휴리스틱" 우선

fallback이 반드시 호출 그래프 DFS일 필요는 없다. 1차 fallback은 **파일 경로 + 레이어 휴리스틱 정렬**(controller → service → domain → repository → test)로 충분하고 구현이 훨씬 싸다.

호출 그래프 기반 DFS 정렬은 **2차 fallback / 보조 도구**로만 둔다 (아래 9.3). 풀 call graph를 1차 fallback으로 두는 것은 오버엔지니어링이다.

### 9.3 (보조) 호출 그래프 DFS 정렬 — v1 알고리즘 보존

여력이 될 때 보조 신호로 사용. caller → callee DFS, 함수 내 등장 순서 유지, 변경 없는 중간 함수는 bridge(접힌 context)로 표시. 단, 7장의 한계 때문에 단독 신뢰 대상으로 쓰지 않는다.

---

## 10. AI 사용 제약

프롬프트에 포함할 규칙(보조 장치이며, 진짜 안전장치는 8.3 검증):

- 제공된 카드에 없는 symbol을 만들지 말 것
- 제공되지 않은 side effect를 추정하지 말 것
- 테스트가 있다고 지어내지 말 것
- 클러스터 이름은 변경 의도를 짧게 요약할 것
- 요약은 1~3문장으로 제한할 것
- 확실하지 않으면 단정하지 말 것
- merge/split 제안은 명확한 경우에만 할 것

책임 분리:

```
알고리즘 : hunk→symbol 매핑, AST 파싱, 관계 신호 계산, AI 입력 정제, 출력 검증, fallback 정렬
AI       : 클러스터링, 정렬, title/summary, merge/split 제안
```

---

## 11. 최종 처리 흐름

```
PR diff 수집 (open/push 이벤트 시 백그라운드 선분석)
  ↓
changed file / changed range 추출
  ↓
base/head 코드 파싱 (full file AST)
  ↓
hunk → enclosing symbol 매핑   ── 실패 시 → Unclustered changes 버킷
  ↓
symbol 관계 신호 계산 (강/중/약, hub 제외)   ※ diff만 보지 않는다 (2.1)
  ↓
cluster card / change card 정제
  ↓
캐시 조회 (PR + head SHA)
  ├─ hit  → 캐시 결과 사용
  └─ miss → AI 단계 실행 (아래)
        [AI 1] 클러스터링
          ↓
        [AI 2] 클러스터 내/간 정렬   (작은 PR은 1·2 합침)
          ↓
        출력 symbol 화이트리스트 검증
          ├─ 통과 → [AI 3] title/summary(배치) → JIT definition 삽입 → 캐싱
          └─ 실패/초과/타임아웃 → fallback(레이어 휴리스틱, 2차로 DFS)
  ↓
최종 리뷰 UI 표시 (필요 시 단계별 스트리밍)
```

---

## 12. MVP 범위

**포함**
1. diff hunk → enclosing symbol 매핑 (+ 매핑 실패 Unclustered 버킷)
2. 변경 파일 전체 AST 파싱
3. changed symbol 추출
4. symbol 관계 신호 계산 (강/중/약, hub 제외)
5. cluster card 정제 (AI 입력)
6. AI 클러스터링
7. AI 클러스터 내/간 정렬 (작은 PR은 1호출 통합)
8. 출력 화이트리스트 검증
9. AI title / summary (배치)
10. JIT definition 삽입
11. PR + commit SHA 캐싱 + open/push 선분석
12. 레이어 휴리스틱 fallback 정렬
13. AI merge/split 제안 표시 (자동 적용 X)

**제외 (이후)**
- 호출 그래프 DFS 정렬 (2차 fallback로 후속)
- AI 위험도 판단 / 리뷰 포커스 / 테스트 부족 분석
- 리뷰어별 맞춤 정렬 / pairwise ranking
- 복잡한 자동 우선순위 재정렬 / community detection 클러스터링

---

## 13. 핵심 요약

- **정렬·클러스터링은 AI가 한다.** "정확함"보다 "그럴듯함"이 가치이므로 LLM이 더 잘 맞는다.
- **diff만 보지 않는다.** AST·심볼·관계 정보로 변경을 정제해 AI에 주는 것이 정확도·비용을 동시에 지키는 핵심이다.
- **알고리즘은 정렬을 결정하지 않고 AI 입력을 정제하고 출력을 검증한다.** 호출 그래프 DFS는 fallback(그것도 2차)으로만 둔다.
- **AI를 쓰는 순간 결정성·캐싱·레이턴시가 1순위 문제가 된다.** commit SHA 캐싱 + 백그라운드 선분석 + 화이트리스트 검증으로 막는다.

한 줄: **변경을 심볼 단위로 정제해 AI에 맡겨 클러스터링·정렬하고, 알고리즘은 입력 정제·출력 검증·fallback을 담당한다.**

---

## 우리 앱(Loupe) 맥락 번역 (로컬 git 데스크탑)

- v2는 "GitHub PR open/push 이벤트 선분석"을 가정하나, Loupe는 **로컬 git 데스크탑 앱**이다. → "PR open/push 선분석"은 **"리뷰 시작(Onboarding에서 base/target 확정) 시 백그라운드 분석 + commit(head) SHA 캐싱"**으로 번역한다. 캐싱 키(head SHA)는 동일하게 작동한다.
- AI 호출 방식: 정렬/클러스터링 3단계는 **단일 구조화 호출**(에이전트 아님). 질문 기능만 에이전트 위임(코드베이스 탐색). 인증 = onboarding의 setup-token → Anthropic API 직접 호출(BYO API 키도 동일 경로). 헤비 유저는 BYO 키 권장(구독 rate limit 완화).
- 기존 엔진 1단계(git2 3-dot diff, hunk→symbol 매핑, tree-sitter full AST, 심볼 추출)는 v2 MVP 1~3에 그대로 해당 → **재사용**. 정렬만 `cards.rs`의 결정적 순서 → AI 클러스터링/정렬로 교체(`order_cards()`가 교체점).

---

## v2.1 정제 (2026-06-19) — 강한관계 seed + AI 보정 (하이브리드)

> 4장 수정. "알고리즘은 관계신호 힌트만 주고 AI가 백지에서 클러스터링"을 → **"알고리즘이 강한 관계로 1차 seed 클러스터를 확정하고, AI는 seed를 출발점으로 보정"**으로 정제. 토큰·속도 이점.

### 왜 바꾸나
명확한 관계(같은 클래스 메서드, 직접 호출 체인, `Repository↔Entity`, signature 타입)는 알고리즘이 **확실하게** 묶을 수 있다. 이걸 AI에 다시 시키는 건 토큰 낭비. 알고리즘이 1차 묶으면 AI 입력이 "개별 심볼 N개" → "seed 묶음 M개(M≪N)"로 압축되고, AI 작업이 "처음부터 묶기" → "검토+보정"으로 가벼워진다(빠른 모델로 충분).

### 핵심 한계 (왜 "끊긴 곳만 AI"는 안 되나)
알고리즘은 **"끊긴 곳"을 식별할 수 없다.** 이벤트 `publish→subscribe`가 끊긴 건지 두 심볼이 무관한 건지, 알고리즘 눈엔 둘 다 "관계 없음"으로 똑같이 보인다(끊김 = 정의상 안 보임). 따라서 AI에게 "끊긴 데만 봐"는 불가능 → **AI는 전체 seed를 한 번 훑어야** 한다. ("AI는 끊긴 데만"이 아니라 "AI는 전체 seed를 훑되 seed를 출발점으로".)

### 두 레이어의 "훑기" (토큰 모순 방지)
- **알고리즘**: 코드베이스(full AST + 참조관계)를 훑어 strong-seed + 정제 맥락 생성.
- **AI**: 정제된 **전체 seed**를 훑어 보정. raw 코드베이스 통째는 AI 입력에 넣지 않음 — cluster card의 시그니처/관계/요약으로 맥락 제공(토큰 방어). (더 깊은 코드 탐색은 "질문하기" 에이전트 기능, 클러스터링과 분리.)

### 수정된 파이프라인 (4장 / ② 단계 대체)
```
② 관계신호 계산 (강/약, hub 제외)
     ↓
②.5 strong-seed 1차 클러스터  ← 신규: 강한 관계만으로 알고리즘이 확정적 묶음
     (같은클래스 / 직접호출 / Repo↔Entity / signature타입. 약한 관계로는 seed 안 만듦.)
     ↓
③ cluster card 정제 — 입력 단위 = 개별 심볼 → seed 묶음
     ↓
④ AI 보정: seed들을 받아 ┬ 병합 (이벤트 pub↔sub, 인터페이스↔구현 등 정적분석이 끊는 의미연결)
                          ├ 분리 (한 seed가 사실 두 동작)
                          ├ 정렬 (클러스터 내/간 흐름순)
                          └ 이름·요약
     ↓
⑤ 화이트리스트 검증 + 캐싱
```

### 규칙
- **seed는 *제안*일 뿐.** AI 프롬프트에 "seed는 알고리즘 추정이며 자유롭게 재구성(병합/분리/이동) 가능"을 명시 → 앵커링 편향 방지.
- **강한 관계만 seed.** 약한 관계(import-only, util, logger, 같은 파일)로 묶으면 틀린 seed 양산 → 약한 신호는 seed 근거 아님(AI 판단 재료로만).
- **결정성 유지.** seed 생성은 순수 함수(결정적) + head SHA 캐싱. AI 보정은 head SHA layout 캐싱으로 흡수.
- **fallback**: AI 실패 시 strong-seed 1차 클러스터 + 레이어 휴리스틱 정렬만으로도 "엉성하지만 동작"(9장 fallback이 더 튼튼해짐 — seed가 이미 일부 묶여 있으므로).
