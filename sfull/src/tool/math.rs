use s20_tool_refactor_macros::tool;

#[tool(name = "add", description = "Add two integers.")]
pub async fn add(
    #[schemars(description = "Left integer operand.")] a: i32,
    #[schemars(description = "Right integer operand.")] b: i32,
) -> i32 {
    a + b
}
