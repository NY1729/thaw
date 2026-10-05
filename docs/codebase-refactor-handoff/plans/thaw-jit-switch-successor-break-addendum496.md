# Switch successor review correction: break stack — unit496

Corrects bounded review /tmp/thaw-jit-switch-successor-review496.md SHA ef50a256309854469c3df74074f439cd3a147391f52efcf7674e63aabbd89958, whose bytes remain unchanged. Source authority: analysis a8ff14cbefa11f8691ba4c4e9fc45092bd611742adfe58943889fc773dcb4ec8; codegen780409782bbb29b7d9c3737ecee69c129acdb61bcaab36eed5b2ac3c3a2b198c. No execution/product edits.

Concrete missed mismatch: SwitchBreak permits runtime depth>=base_depth in codegen and jumps to the exit target; temporary XMMs above the switch base are not live afterward. Analyzer instead stores full stack as its break exit. SwitchEnd joins those full stacks and pops one top slot, which can remove a temporary rather than the discriminant or falsely reject a valid join.

Correct break transfer: require stack length>=switch base length, clone mutated prefix through base length only, push that normalized exit, then mark unreachable. Do not restore original base values: mutations to live locals/prefix must survive. Normal reachable body-end must require exact base length before collecting. At SwitchEnd join normalized exits then pop last base slot, the discriminant, once. ResultReturn and early/throw exits use their own targets and must not be forcibly normalized as switch breaks.

Unrun control: break with live temporary above discriminant plus syntactic cleanup in unreachable suffix, multiple break exits of different temporary depths, mutated prefix aliases, nested switch breaks. Original source review's no-new-CFG-defect conclusion was too broad for this omitted path; bounded successor verdict is HOLD until this exact normalization is source-corrected. Root and author notified.
