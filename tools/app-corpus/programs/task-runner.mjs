// Task runner: child_process.execFile / spawn of node -e and echo with deterministic output.
import { execFile, spawn } from 'node:child_process';
import { promisify } from 'node:util';

const exec = promisify(execFile);

const tasks = [
  { name: 'echo', cmd: 'echo', args: ['hello', 'tasks'] },
  { name: 'node-eval', cmd: process.execPath, args: ['-e', 'console.log(2 + 3); console.error("warn")'] },
  { name: 'node-exit', cmd: process.execPath, args: ['-e', 'process.exit(7)'] },
  { name: 'missing', cmd: 'definitely-not-a-command-xyz', args: [] },
];

for (const t of tasks) {
  try {
    const { stdout, stderr } = await exec(t.cmd, t.args);
    console.log(t.name, 'ok', JSON.stringify(stdout), JSON.stringify(stderr));
  } catch (e) {
    console.log(t.name, 'failed', e.code, e.stdout ?? '');
  }
}

const out = await new Promise((resolve) => {
  const p = spawn(process.execPath, ['-e', 'process.stdin.pipe(process.stdout)'], { stdio: ['pipe', 'pipe', 'inherit'] });
  let s = '';
  p.stdout.on('data', (d) => (s += d));
  p.on('close', (code) => resolve({ code, s }));
  p.stdin.end('piped through child\n');
});
console.log(out);
