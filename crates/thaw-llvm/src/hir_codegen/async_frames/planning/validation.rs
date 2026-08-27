impl<'ctx> HirCompiler<'ctx> {
    fn stmt_contains_frame_unsupported(stmt: &HirStmt) -> bool {
        match stmt {
            HirStmt::Let(..) | HirStmt::Return(..) | HirStmt::Throw(..) | HirStmt::Try(..) => true,
            HirStmt::If(_, then_body, else_body) => {
                if matches!(
                    (then_body.as_slice(), else_body.as_slice()),
                    ([HirStmt::Throw(_)], [])
                ) {
                    return false;
                }
                then_body.iter().any(Self::stmt_contains_frame_unsupported)
                    || else_body.iter().any(Self::stmt_contains_frame_unsupported)
            }
            HirStmt::While(_, body) => body.iter().any(Self::stmt_contains_frame_unsupported),
            _ => false,
        }
    }

}
