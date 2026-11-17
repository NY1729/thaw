export declare class EventEmitter { constructor(options?: any); on(event: any, listener: (...args: any[]) => void): EventEmitter; on(event: any, listener: (first: any, second?: any) => void): EventEmitter; addListener(event: any, listener: (...args: any[]) => void): EventEmitter; once(event: any, listener: (...args: any[]) => void): EventEmitter; once(event: any, listener: (first: any, second?: any) => void): EventEmitter; off(event: any, listener: (...args: any[]) => void): EventEmitter; removeListener(event: any, listener: (...args: any[]) => void): EventEmitter; emit(event: any, ...args: any[]): boolean; listenerCount(event: any): number; removeAllListeners(event?: any): EventEmitter; }
export default EventEmitter;
export declare function once(emitter: EventEmitter, event: any, options?: any): Promise<any[]>;
export declare function on(emitter: EventEmitter, event: any, options?: any): AsyncIterable<any[]>;
export declare const errorMonitor: symbol;
export declare const captureRejectionSymbol: symbol;
export declare function getEventListeners(emitter: EventEmitter, event: any): any[];
export declare function getMaxListeners(emitter: EventEmitter): number;
export declare function setMaxListeners(count: number, emitter: EventEmitter): void;
