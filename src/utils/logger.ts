/**
 * Application-wide logging utility
 * Provides structured logging with different levels and optional log storage
 */

export enum LogLevel {
  DEBUG = 0,
  INFO = 1,
  WARN = 2,
  ERROR = 3,
  NONE = 4,
}

export interface LogEntry {
  timestamp: Date
  level: LogLevel
  context: string
  message: string
  data?: any
}

class Logger {
  private static instance: Logger
  private logs: LogEntry[] = []
  private maxLogs = 1000
  private currentLevel: LogLevel = import.meta.env.DEV ? LogLevel.DEBUG : LogLevel.WARN

  private constructor() {}

  static getInstance(): Logger {
    if (!Logger.instance) {
      Logger.instance = new Logger()
    }
    return Logger.instance
  }

  setLevel(level: LogLevel) {
    this.currentLevel = level
  }

  getLevel(): LogLevel {
    return this.currentLevel
  }

  private log(level: LogLevel, context: string, message: string, data?: any) {
    if (level < this.currentLevel) {
      return
    }

    const entry: LogEntry = {
      timestamp: new Date(),
      level,
      context,
      message,
      data,
    }

    this.logs.push(entry)
    if (this.logs.length > this.maxLogs) {
      this.logs.shift()
    }

    // In development, also output to console for convenience
    if (import.meta.env.DEV) {
      const levelName = LogLevel[level]
      const logMessage = `[${levelName}] [${context}] ${message}`

      switch (level) {
        case LogLevel.DEBUG:
          // eslint-disable-next-line no-console
          console.debug(logMessage, data || '')
          break
        case LogLevel.INFO:
          // eslint-disable-next-line no-console
          console.info(logMessage, data || '')
          break
        case LogLevel.WARN:
          // eslint-disable-next-line no-console
          console.warn(logMessage, data || '')
          break
        case LogLevel.ERROR:
          // eslint-disable-next-line no-console
          console.error(logMessage, data || '')
          break
      }
    }
  }

  debug(context: string, message: string, data?: any) {
    this.log(LogLevel.DEBUG, context, message, data)
  }

  info(context: string, message: string, data?: any) {
    this.log(LogLevel.INFO, context, message, data)
  }

  warn(context: string, message: string, data?: any) {
    this.log(LogLevel.WARN, context, message, data)
  }

  error(context: string, message: string, data?: any) {
    this.log(LogLevel.ERROR, context, message, data)
  }

  getLogs(level?: LogLevel): LogEntry[] {
    if (level !== undefined) {
      return this.logs.filter(log => log.level === level)
    }
    return [...this.logs]
  }

  clearLogs() {
    this.logs = []
  }

  exportLogs(): string {
    return JSON.stringify(this.logs, null, 2)
  }
}

// Export singleton instance methods
const logger = Logger.getInstance()

export const setLogLevel = (level: LogLevel) => logger.setLevel(level)
export const getLogLevel = () => logger.getLevel()
export const debug = (context: string, message: string, data?: any) => logger.debug(context, message, data)
export const info = (context: string, message: string, data?: any) => logger.info(context, message, data)
export const warn = (context: string, message: string, data?: any) => logger.warn(context, message, data)
export const error = (context: string, message: string, data?: any) => logger.error(context, message, data)
export const getLogs = (level?: LogLevel) => logger.getLogs(level)
export const clearLogs = () => logger.clearLogs()
export const exportLogs = () => logger.exportLogs()

export default logger