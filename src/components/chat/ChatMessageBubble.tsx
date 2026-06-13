import React from 'react'
import type { ChatMessageVM } from '../../hooks/useAIChat'
import { renderMarkdownLite } from '../../utils/markdownLite'
import { ToolActivityChips } from './ToolActivityChips'

/**
 * One chat message. Memoized so that during streaming only the actively
 * updating message re-renders per flush, not the whole transcript.
 */
export const ChatMessageBubble: React.FC<{ message: ChatMessageVM }> = React.memo(({ message }) => {
  const isUser = message.role === 'user'
  const showBubble = isUser || !!message.error || message.text.length > 0 || message.streaming
  return (
    <div className={`chat-msg ${isUser ? 'user' : 'bot'}`}>
      <div className="av">{isUser ? 'ME' : <img src="/wf-ds/chatgpt-bot-avatar.webp" alt="" />}</div>
      <div className="msg-col">
        <span className="msg-sender">{isUser ? 'You' : 'WF Assistant'}</span>
        {message.tools.length > 0 && <ToolActivityChips tools={message.tools} />}
        {showBubble && (
          <div className={`bubble${message.streaming ? ' streaming' : ''}`}>
            {message.error ? (
              <span className="chat-error">
                <i className="fa-solid fa-triangle-exclamation" aria-hidden="true" /> {message.error}
              </span>
            ) : isUser ? (
              <div style={{ whiteSpace: 'pre-wrap' }}>{message.text}</div>
            ) : message.text ? (
              renderMarkdownLite(message.text)
            ) : message.tools.length === 0 ? (
              <i className="fa-solid fa-circle-notch fa-spin" aria-hidden="true" />
            ) : null}
          </div>
        )}
      </div>
    </div>
  )
})

ChatMessageBubble.displayName = 'ChatMessageBubble'
