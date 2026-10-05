# JIT consumer operand proof source review — unit496

Live candidate `/tmp/thaw-luna-jit-result-chain/candidate`; managed baseline f4e2faf98b6973306ac634f2d78968904b846480. Source only, no product edits/execution. Read hashes analysis.rsb6eaa31615cf899ff3ffaeb34a1f25d4682277ced28f0e1508b8a28ccaf1dd14; parser.rs9ac6122b5c9be174741abe4ec8c460b62ac6dcab6569468b78f296b1e1e1756a; codegen.rsfa628f3b77d1a03e78980cfb6024459614f2bf28f927a0bf05943efff3ffb8e5. Mutable read pins; full acceptance HOLD.

## Concrete consumed-kind hole

Parser maps scalar argument tokens to Argument with Number/Boolean/String annotation and maps strlen directly to StringLength without validating an operand stack. The analyzer generic transfer asks callback_operation_kind only for consumed count and output family. It checks stack depth, truncates, then pushes output. Therefore Number input followed by StringLength can receive a Number result proof; the emitter actually calls string_length on the raw Number word. Runtime string_length decodes String representation. Parser does not prevent this exact semantic path. Similar generic String conversions, comparisons, concat, slices and searches lack consumed-family validation. TagMaybe's explicit transfer fixes only its own operation.

Required smallest complete change: extend the existing operation transfer description with ordered operand requirements, validate the actual consumed suffix BEFORE truncation, then compute output. Unary String pointer consumers require String; StringConcat/Compare/SameValue require both String; String/index routines require String then numeric index; dynamic conversions/compare require Dynamic; native reference consumers require appropriate reference representation. Numeric/coercion ops must describe accepted inputs and actual normalization, not merely output Number. Unknown input cannot acquire a pointer-family proof from the consumer opcode. Keep structured control's full-stack joins and source annotations authoritative where actually validated.

## Broad container kind is insufficient

argument_source_kind and local_source_kind collapse rn/rb/rs to ArrayReference and dn/db/ds to DictionaryReference. These layouts share buffers but element words differ: a numeric read and a String pointer read are not interchangeable. Broad ArrayReference can prove array length/layout access, but cannot prove StringArrayGet's returned payload is a valid String. DictionaryReference has the analogous issue. Dynamic array wrappers additionally carry a runtime tag.

Preserve minimal element-family metadata from existing parser tokens and producer operations only where consumers need it: number/boolean/string array and dictionary evidence (and dynamic wrapper evidence). This may be a refinement attached to existing callback kind/source record rather than a new type system. Container mutation/conversion outputs update the same evidence. Joining incompatible families loses exact evidence or routes through existing Dynamic conversion; never silently select String. Tuple/object field offsets require actual producer layout evidence, not only requested result field token; their existing HIR/compiler descriptors are the relevant source.

Inspect runtime validation: array_data proves broad handle/layout, not primitive element String-vs-Number family. Reading a registered array handle therefore does not replace element-kind proof. Dynamic primitive routines validate their wrapper tag, but the proof/emitter must still obey accepted subtype/error contract.

## Caller environment closure

Token annotations alone are not proof of actual callback inputs. primitive map source encoding determines argument0 family; a callback token asserting String cannot reinterpret a Number source. compile_jit_callback now uses full callback result proof, but must supply/check actual builtins argument environment: raw element family, index Number, array reference subtype, tagged argument pairs and reducer accumulator family. Capture packet transports presence only, not kind; frontend capture descriptor or validated compile-time environment must supply capture family. Validate token annotations against that environment, and apply the same contract recursively. Preserve legacy valid modes through inference from actual caller environment; do not reject supported frontend syntax to avoid plumbing. Unknown opaque inputs use existing safe runtime/fallback route until evidence exists, not invented pointer proof.

## Optional policy

Maybe family is not automatically a dereferenceable pointer. PresentConditionalStart narrows on its present branch. Other Maybe consumers require explicit presence-sensitive conversion or TypeError/optional semantics according to opcode producer contract. StringToNumber can convert optional undefined/null only through its defined conversion route; StringLength member access on absent must not decode raw bits or invent zero. Dynamic conversion can box absence before wrapper access. Array/dictionary optional lookup preserves its state until a consumer deliberately coerces. Validate both requested input family and implemented state handling before certifying output.

Minimal owners: parser/source evidence, analysis transfer table and joins, actual codegen operand normalization, callback compiler caller environment, relevant CLI producer annotations. Reuse existing runtime conversions, roots and error channel. Add unrun controls for wrong-family scalar pointer consumer, container element-family mismatch, caller-builtins annotation mismatch, captures, optional present/error branches, loops/locals preserving family, and recursive environment. No controls executed.

Marker: FULL_CONSUMED_KIND_AND_CALLER_INPUT_PROOF_HOLD.
