import { marked } from "marked";
function main(): void {
  console.log(marked.parse("# Title\n\nSome *emphasis* and `code`.\n\n- a\n- b\n"));
}
