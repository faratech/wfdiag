import React from 'react'
import type { ChatMessageVM } from '../../hooks/useAIChat'
import { renderMarkdownLite } from '../../utils/markdownLite'
import { ToolActivityChips } from './ToolActivityChips'
import { StagedActionCards } from './StagedActionCards'

/**
 * One chat message. Memoized so that during streaming only the actively
 * updating message re-renders per flush, not the whole transcript.
 */
export const ChatMessageBubble: React.FC<{ message: ChatMessageVM }> = React.memo(({ message }) => {
  const isUser = message.role === 'user'
  const showBubble = isUser || !!message.error || message.text.length > 0 || message.streaming
  const providerMeta = message.providerUse
    ? `${message.providerUse.providerId.replace(/_/g, ' ')} · ${message.providerUse.executionClass.replace(/_/g, ' ')}`
    : null
  return (
    <article className={`chat-msg ${isUser ? 'user' : 'bot'}`} aria-label={`${isUser ? 'Your' : 'Assistant'} message`}>
      <div className="av" aria-hidden="true">{isUser ? 'ME' : <img src="/wf-ds/chatgpt-bot-avatar.webp" alt="" />}</div>
      <div className="msg-col">
        <span className="msg-sender">
          {isUser ? 'You' : 'WF Assistant'}
          {!isUser && providerMeta && <span className="msg-provider">{providerMeta}</span>}
        </span>
        {message.tools.length > 0 && <ToolActivityChips tools={message.tools} />}
        {!!message.stagedProposals?.length && <StagedActionCards proposals={message.stagedProposals} />}
        {!isUser && message.streaming && (
          <div className="ai-activity chat-reasoning" role="status">
            <span><i className="fa-solid fa-circle-notch fa-spin" aria-hidden="true" /> Reasoning</span>
          </div>
        )}
        {showBubble && (
          <div className={`bubble${message.streaming ? ' streaming' : ''}`}>
            {message.error ? (
              <span className="chat-error" role="alert">
                <i className="fa-solid fa-triangle-exclamation" aria-hidden="true" /> {message.error}
              </span>
            ) : isUser ? (
              <div style={{ whiteSpace: 'pre-wrap' }}>{message.text}</div>
            ) : message.text ? (
              renderMarkdownLite(message.text)
            ) : message.tools.length === 0 ? (
              <i className="fa-solid fa-circle-notch fa-spin" aria-hidden="true" />
            ) : null}
            {!isUser && message.finishReason === 'length' && (
              <div className="chat-incomplete" role="status">Response stopped at the provider's output limit.</div>
            )}
            {!isUser && message.finishReason === 'tool_budget' && (
              <div className="chat-incomplete" role="status">Tool budget reached; this answer uses the evidence already gathered.</div>
            )}
          </div>
        )}
      </div>
    </article>
  )
})

ChatMessageBubble.displayName = 'ChatMessageBubble'
