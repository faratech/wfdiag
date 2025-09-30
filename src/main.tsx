import React from 'react'
import ReactDOM from 'react-dom/client'
import { FluentProvider } from '@fluentui/react-components'
import App from './App'
import { wfDarkTheme } from './theme'
import './index.css'

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <FluentProvider theme={wfDarkTheme}>
      <App />
    </FluentProvider>
  </React.StrictMode>,
)