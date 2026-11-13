const tag = new WebAssembly.Tag({ parameters: ['i32', 'f64'] });
const e = new WebAssembly.Exception(tag, [1, 2.5]);
console.log(e.getArg(tag, 0), e.getArg(tag, 1), e.is(tag), e.is(new WebAssembly.Tag({ parameters: [] })));
console.log(e instanceof Error, Object.prototype.toString.call(tag), Object.prototype.toString.call(e), tag instanceof WebAssembly.Tag);
try { e.getArg(tag, 5); } catch (er) { console.log(er.name, er.message); }
try { new WebAssembly.Tag({}); } catch (er) { console.log(er.name, er.message); }
console.log(WebAssembly.JSTag instanceof WebAssembly.Tag, typeof WebAssembly.compileStreaming, WebAssembly.compileStreaming.length);
