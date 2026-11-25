// CLI tool: util.parseArgs over a fixed argv, subcommands and exit codes.
import { parseArgs } from 'node:util';

const options = {
  verbose: { type: 'boolean', short: 'v' },
  name: { type: 'string', short: 'n', default: 'world' },
  count: { type: 'string', short: 'c', multiple: true },
  help: { type: 'boolean', short: 'h' },
};

function run(argv) {
  let parsed;
  try {
    parsed = parseArgs({ args: argv, options, allowPositionals: true });
  } catch (e) {
    console.error('error:', e.code);
    return 2;
  }
  const { values, positionals } = parsed;
  if (values.help) {
    console.log('usage: tool [-v] [-n NAME] [-c N]... <cmd>');
    return 0;
  }
  const [cmd, ...rest] = positionals;
  switch (cmd) {
    case 'greet':
      console.log(`hello ${values.name}${values.verbose ? '!' : ''}`);
      return 0;
    case 'sum':
      console.log((values.count ?? []).map(Number).reduce((a, b) => a + b, 0), rest.length);
      return 0;
    default:
      console.error(`unknown command: ${cmd}`);
      return 1;
  }
}

const codes = [
  run(['greet', '-n', 'thaw', '-v']),
  run(['--count', '1', '-c', '2', '--count=3', 'sum', 'a', 'b']),
  run(['--bogus']),
  run(['-h']),
  run(['nope']),
];
console.log(codes);
process.exitCode = Math.max(...codes);
