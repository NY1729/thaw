impl NumericProgram {

    fn required_args(&self) -> usize {
        self.0
            .iter()
            .filter_map(|value| match value {
                NumericValue::Argument(index) => Some(*index as usize + 1),
                NumericValue::DynamicArgument(index) => Some(*index as usize + 2),
                _ => None,
            })
            .max()
            .unwrap_or(0)
    }
}
