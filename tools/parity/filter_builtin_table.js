#!/usr/bin/env node
// Writes crates/thaw-js/src/builtin_table_embed.tsv: the rows of builtin_table.tsv that thaw-js applies at
// startup, i.e. the table minus the rows listed in builtin_table_missing.txt (thaw-js lacks their owner or key,
// so applying them is a no-op). Rows of a root thaw-js does not provide at all drop out with it.
// The WI-1b test checks that the embed is exactly this filter's output (no Node needed there).
// Usage: node tools/parity/filter_builtin_table.js   (run after regenerating the table and the missing list)
'use strict';
const fs = require('fs');
const path = require('path');

const src = path.join(__dirname, '..', '..', 'crates', 'thaw-js', 'src');
const full = fs.readFileSync(path.join(src, 'builtin_table.tsv'), 'utf8').split('\n');
const missing = new Set(
  fs.readFileSync(path.join(src, 'builtin_table_missing.txt'), 'utf8')
    .split('\n')
    .filter((l) => l !== '' && !l.startsWith('#')),
);
const rows = full.slice(1).filter((line) => line !== '');
const kept = rows.filter((line) => !missing.has(line.split('\t').slice(0, 2).join('\t')));
const text = [full[0], ...kept].join('\n') + '\n';
fs.writeFileSync(path.join(src, 'builtin_table_embed.tsv'), text);
console.log(`builtin_table_embed.tsv: ${kept.length} of ${rows.length} rows, ${Buffer.byteLength(text)} bytes`);
