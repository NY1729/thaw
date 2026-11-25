// CLI tool: consume piped stdin, count words, uppercase.
let data = '';
process.stdin.setEncoding('utf8');
for await (const chunk of process.stdin) data += chunk;
const lines = data.split('\n').filter(Boolean);
console.log('lines', lines.length);
console.log('words', lines.join(' ').split(/\s+/).length);
for (const [i, l] of lines.entries()) console.log(String(i + 1).padStart(3), l.toUpperCase());
if (lines.length === 0) process.exitCode = 4;
