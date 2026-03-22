// Opt-in logging for telex internals.
//
// Silent by default — nothing hits the console unless a logger is installed.
// Test harnesses can set `setTelexLogger(console)` to get full visibility.

export interface TelexLogger {
  debug(msg: string, ...args: unknown[]): void;
  error(msg: string, ...args: unknown[]): void;
}

let currentLogger: TelexLogger | null = null;

export function setTelexLogger(logger: TelexLogger | null): void {
  currentLogger = logger;
}

export function telexLogger(): TelexLogger | null {
  return currentLogger;
}
