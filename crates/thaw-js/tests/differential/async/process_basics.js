console.log(typeof process.version, process.platform, Array.isArray(process.argv), typeof process.env, typeof process.cwd());
console.log(typeof process.hrtime()[0], process.hrtime().length, typeof process.uptime(), typeof process.pid);
console.log(process.versions.node === process.version.slice(1), typeof process.memoryUsage().rss);
