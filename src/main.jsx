import React from 'react';
import ReactDOM from 'react-dom/client';

// Design tokens first (order mirrors the prototype's styles.css), then globals.
import './styles/tokens/fonts.css';
import './styles/tokens/colors.css';
import './styles/tokens/typography.css';
import './styles/tokens/spacing.css';
import './styles/tokens/motion.css';
import './styles/global.css';

import App from './App.jsx';

ReactDOM.createRoot(document.getElementById('root')).render(<App />);
