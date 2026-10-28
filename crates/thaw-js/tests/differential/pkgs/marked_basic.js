// pkg: marked
const { marked, Marked } = require('marked');
const log = (...a) => console.log(...a);
log(marked.parse('# Title\n\nSome *emphasis* and **strong** text with `code`.'));
log(marked.parse('- a\n- b\n  - c\n\n1. one\n2. two'));
log(marked.parseInline('inline *only* [link](http://x.y "t")'));
log(marked.parse('```js\nconst a = 1;\n```\n\n> quote\n\n---\n\n| a | b |\n|---|---|\n| 1 | 2 |'));
const m = new Marked({ gfm: false, breaks: true });
log(m.parse('line1\nline2 ~~strike~~'));
log(typeof marked.use, typeof marked.lexer, marked.lexer('# a').length, marked.lexer('# a')[0].type);
log(marked('html <b>passes</b> & escapes <script>x</script>'));
