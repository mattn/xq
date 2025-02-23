use std::error::Error;
use xq::{module_loader::PreludeLoader, run_query, Value};

#[macro_export]
macro_rules! test {
    ($name: ident, $query: expr, $input: expr, $output: expr) => {
        #[test]
        fn $name() -> Result<(), Box<dyn std::error::Error>> {
            crate::common::run_test($query, $input, $output)
        }
    };
}

pub(crate) fn run_test(query: &str, input: &str, output: &str) -> Result<(), Box<dyn Error>> {
    let input: Vec<_> = serde_json::de::Deserializer::from_str(input)
        .into_iter::<Value>()
        .collect::<Result<_, serde_json::Error>>()?;
    let expected: Vec<_> = serde_json::de::Deserializer::from_str(output)
        .into_iter::<Value>()
        .collect::<Result<_, serde_json::Error>>()?;
    let output = run_query(query, input.into_iter(), &PreludeLoader())?
        .collect::<Result<Vec<Value>, _>>()?;
    if expected != output {
        eprintln!("{:?} {:?}", expected, output);
    }
    assert_eq!(expected, output);
    Ok(())
}
