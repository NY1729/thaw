export declare class Buffer { static from(value: Json, encoding?: string): Buffer; static alloc(size: number, fill?: Json, encoding?: string): Buffer; static byteLength(value: string, encoding?: string): number; static concat(values: Buffer[]): Buffer; toString(encoding?: string): string; subarray(start?: number, end?: number): Buffer; readUInt16LE(offset?: number): number; readInt32BE(offset?: number): number; writeUInt16LE(value: number, offset?: number): number; writeInt32BE(value: number, offset?: number): number; }
export declare const SlowBuffer: any;
export declare const Blob: any;
export declare const File: any;
export declare function atob(data: string): string;
export declare function btoa(data: string): string;
export declare function byteLength(value: string, encoding?: string): number;
export declare function isUtf8(argsArray: any): any;
export declare function isAscii(argsArray: any): any;
export declare function transcode(argsArray: any): any;
export declare const constants: any;
export declare const kMaxLength: number;
export declare const INSPECT_MAX_BYTES: number;
