export declare class URLSearchParams { constructor(init?: string); append(name: string, value: string): void; set(name: string, value: string): void; get(name: string): string | null; getAll(name: string): string[]; }
export declare class URL { constructor(input: string, base?: string); hostname: string; pathname: string; searchParams: URLSearchParams; }
export declare function parse(url: string, parseQueryString?: boolean, slashesDenoteHost?: boolean): any;
export declare function format(urlObject: any, options?: any): string;
export declare function pathToFileURL(path: string, options?: any): JsValue;
export declare function fileURLToPath(url: string, options?: any): string;
export declare function fileURLToPath(url: JsValue, options?: any): string;
export declare function urlToHttpOptions(url: JsValue): any;
export declare function resolve(from: string, to: string): string;
export declare function domainToASCII(domain: string): string;
export declare function domainToUnicode(domain: string): string;
