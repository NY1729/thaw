export declare function stringify(object: Json, separator?: string, assignment?: string, options?: Json): string;
export declare function encode(object: Json, separator?: string, assignment?: string, options?: Json): string;
export declare function parse(text: string, separator?: string, assignment?: string, options?: Json): Json;
export declare function decode(text: string, separator?: string, assignment?: string, options?: Json): Json;
export declare function escape(value: Json): string;
export declare function unescape(value: Json): string;
