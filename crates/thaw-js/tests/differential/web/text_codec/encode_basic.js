const enc = new TextEncoder();
console.log(enc.encoding);
console.log(JSON.stringify(Array.from(enc.encode('héllo😀'))));
console.log(enc.encode('').length);
console.log(JSON.stringify(Array.from(enc.encode()))); 
console.log(Object.prototype.toString.call(enc.encode('a')));
console.log(enc.encode('a').constructor === Uint8Array);
console.log(JSON.stringify(Array.from(enc.encode(undefined))));
console.log(JSON.stringify(Array.from(enc.encode(42))));
