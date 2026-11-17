export interface ParsedPath { root: string; dir: string; base: string; ext: string; name: string; }
export declare function resolve(...paths: string[]): string;
export declare function join(...paths: string[]): string;
export declare function dirname(path: string): string;
export declare function basename(path: string, suffix?: string): string;
export declare function extname(path: string): string;
export declare function normalize(path: string): string;
export declare function relative(from: string, to: string): string;
export declare function isAbsolute(path: string): boolean;
export declare function parse(path: string): ParsedPath;
export declare function format(pathObject: Json): string;
export declare function toNamespacedPath(path: string): string;
export declare const sep: string;
export declare const delimiter: string;
export declare const posix: any;
export declare const win32: any;
