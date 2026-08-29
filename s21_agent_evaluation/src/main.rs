use s21_agent_evaluation::{EvalRunner, default_cases, get_llm_client};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let runner = EvalRunner::new(get_llm_client()?);
    let mut summary = Vec::new();

    for case in default_cases() {
        println!("Running: {}", case.name);

        match runner.run_case(&case).await {
            Ok((_run, result)) => {
                let status = if result.passed { "PASS" } else { "FAIL" };
                println!("  {status} {:.2}", result.score);
                for reason in result.reason.lines() {
                    println!("  {reason}");
                }
                summary.push((case.name, result));
            }
            Err(error) => {
                println!("  ERROR {error:#}");
                summary.push((
                    case.name,
                    s21_agent_evaluation::EvalResult {
                        passed: false,
                        score: 0.0,
                        reason: format!("evaluation error: {error:#}"),
                    },
                ));
            }
        }

        println!();
    }

    println!("Evaluation Summary\n");
    for (name, result) in &summary {
        let status = if result.passed { "PASS" } else { "FAIL" };
        println!("{name:<14} {status:<4}  {:.2}", result.score);
    }

    let passed = summary.iter().filter(|(_, result)| result.passed).count();
    println!("\n{passed} / {} passed", summary.len());

    Ok(())
}
