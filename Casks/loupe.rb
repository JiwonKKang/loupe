cask "loupe" do
  version "0.1.2"
  sha256 :no_check # ad-hoc / self-hosted release; swap for a real digest per version if you prefer

  url "https://github.com/JiwonKKang/loupe/releases/download/v#{version}/Loupe_#{version}_universal.dmg",
      verified: "github.com/JiwonKKang/loupe/"
  name "Loupe"
  desc "Human-first code review desktop app (data-flow ordered, AI-assisted)"
  homepage "https://github.com/JiwonKKang/loupe"

  depends_on macos: :big_sur

  app "Loupe.app"

  # The build is ad-hoc signed (no Apple Developer certificate). Strip the
  # quarantine flag on install so Gatekeeper doesn't block the first launch.
  postflight do
    system_command "/usr/bin/xattr",
                   args: ["-dr", "com.apple.quarantine", "#{appdir}/Loupe.app"]
  end

  uninstall quit: "com.jiwon.loupe"

  zap trash: [
    "~/Library/Application Support/com.jiwon.loupe",
  ]
end
