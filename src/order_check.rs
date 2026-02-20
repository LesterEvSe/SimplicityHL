use std::collections::HashSet;
use crate::parse;
use crate::str::{FunctionName, Identifier};

#[derive(Debug, Clone)]
pub enum OrderError {
    ForwardReference {
        user: Identifier,
        used: Identifier,
    },
}

pub struct OrderValidator {
    pub defined_functions: HashSet<FunctionName>,
}

impl OrderValidator {
    pub fn new() -> Self {
        Self {
            defined_functions: HashSet::new(),
        }
    }

    pub fn check_stream<'a, I>(&mut self, items: I) -> Result<(), OrderError>
    where
        I: IntoIterator<Item = &'a parse::Item>,
    {
        for item in items {
            self.check_item(item)?;
        }
        Ok(())
    }

    fn check_item(&mut self, item: &parse::Item) -> Result<(), OrderError> {
        match item {
            parse::Item::Function(func) => {
                let func_id = Identifier::from(func.name().clone());

                self.check_expression(func.body(), &func_id)?;

                self.defined_functions.insert(func.name().clone());
            }
            parse::Item::Use(use_decl) => {
                match use_decl.items() {
                    parse::UseItems::Single(elem) => {
                        let func_name = FunctionName::from(elem.as_inner());
                        self.defined_functions.insert(func_name);
                    }
                    parse::UseItems::List(elems) => {
                        for elem in elems {
                            let func_name = FunctionName::from(elem.as_inner());
                            self.defined_functions.insert(func_name);
                        }
                    }
                }
            }
            parse::Item::TypeAlias(_alias) => {
                // self.check_type(alias.target_type())?;
                // self.defined_types.insert(alias.name().clone());
            }
            parse::Item::Module => {
                // for inner_item in module_decl.items() {
                //     self.check_item(inner_item)?;
                // }
            }
        }
        Ok(())
    }

    fn check_expression(&self, expr: &parse::Expression, current_func: &Identifier) -> Result<(), OrderError> {
        match expr.inner() {
            parse::ExpressionInner::Single(single) => self.check_single(single, current_func),
            parse::ExpressionInner::Block(stmts, final_expr) => {
                for stmt in stmts.iter() {
                    match stmt {
                        parse::Statement::Assignment(assign) => {
                            self.check_expression(assign.expression(), current_func)?;
                        }
                        parse::Statement::Expression(e) => {
                            self.check_expression(e, current_func)?;
                        }
                    }
                }
                if let Some(e) = final_expr {
                    self.check_expression(e, current_func)?;
                }
                Ok(())
            }
        }
    }

    fn check_single(&self, single: &parse::SingleExpression, current_func: &Identifier) -> Result<(), OrderError> {
        use parse::SingleExpressionInner as S;
        match single.inner() {
            S::Call(call) => self.check_call(call, current_func),
            S::Expression(e)
            | S::Option(Some(e))
            | S::Either(either::Either::Left(e))
            | S::Either(either::Either::Right(e)) => {
                self.check_expression(e, current_func)
            }
            S::Tuple(args) | S::Array(args) | S::List(args) => {
                for arg in args.iter() {
                    self.check_expression(arg, current_func)?;
                }
                Ok(())
            }
            S::Match(m) => {
                self.check_expression(m.scrutinee(), current_func)?;
                self.check_expression(m.left().expression(), current_func)?;
                self.check_expression(m.right().expression(), current_func)?;
                Ok(())
            }
            S::Boolean(_)
            | S::Decimal(_)
            | S::Binary(_)
            | S::Hexadecimal(_)
            | S::Witness(_)
            | S::Parameter(_)
            | S::Variable(_)
            | S::Option(None) => Ok(()),
        }
    }

    fn check_call(&self, call: &parse::Call, current_func: &Identifier) -> Result<(), OrderError> {
        for arg in call.args().iter() {
            self.check_expression(arg, current_func)?;
        }

        if let parse::CallName::Custom(target_name) = call.name() {
            if !self.defined_functions.contains(target_name) {
                return Err(OrderError::ForwardReference {
                    user: current_func.clone(),
                    used: Identifier::from(target_name.clone()),
                });
            }
        }

        match call.name() {
            parse::CallName::Fold(name, _)
            | parse::CallName::ArrayFold(name, _)
            | parse::CallName::ForWhile(name) => {
                if !self.defined_functions.contains(name) {
                    return Err(OrderError::ForwardReference {
                        user: current_func.clone(),
                        used: Identifier::from(name.clone()),
                    });
                }
            }
            _ => {}
        }

        Ok(())
    }
}

#[cfg(test)]
mod order_tests {
    use super::*;
    use std::fs::{self, File};
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    fn create_simf_file(dir: &Path, rel_path: &str, content: &str) -> PathBuf {
        let full_path = dir.join(rel_path);
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut file = File::create(&full_path).expect("Failed to create file");
        file.write_all(content.as_bytes()).expect("Failed to write content");
        full_path
    }

    fn parse_code(code: &str) -> parse::Program {
        let temp_dir = TempDir::new().unwrap();
        let path = create_simf_file(temp_dir.path(), "test.simf", code);
        crate::driver::parse_and_get_program(&path).expect("Parsing failed")
    }

    fn check_code(code: &str) -> Result<(), OrderError> {
        let program = parse_code(code);
        let mut validator = OrderValidator::new();
        validator.check_stream(program.items())
    }


    #[test]
    fn test_valid_definition_before_use() {
        let code = r#"
            fn helper() { unit }
            fn main() { helper() }
        "#;
        assert!(check_code(code).is_ok());
    }

    #[test]
    fn test_fail_forward_reference() {
        let code = r#"
            fn main() { helper() }
            fn helper() { unit }
        "#;

        let result = check_code(code);
        match result {
            Err(OrderError::ForwardReference { user, used }) => {
                assert_eq!(user.as_ref(), "main");
                assert_eq!(used.as_ref(), "helper");
            }
            _ => panic!("Expected ForwardReference error, got: {:?}", result),
        }
    }

    #[test]
    fn test_fail_recursion() {
        let code = r#"
            fn recursive() { recursive() }
        "#;

        let result = check_code(code);
        match result {
            Err(OrderError::ForwardReference { user, used }) => {
                assert_eq!(user.as_ref(), "recursive");
                assert_eq!(used.as_ref(), "recursive");
            }
            _ => panic!("Recursion should be detected as forward ref, got: {:?}", result),
        }
    }

    #[test]
    fn test_nested_blocks_and_calls() {
        let code = r#"
            fn deep_helper() { unit }

            fn main() {
                {
                    let x: u1 = 0;
                    deep_helper()
                }
            }
        "#;
        assert!(check_code(code).is_ok());
    }

    #[test]
    fn test_fail_nested_forward_ref() {
        let code = r#"
            fn main() {
                {
                    missing_func()
                }
            }
        "#;
        assert!(matches!(
            check_code(code),
            Err(OrderError::ForwardReference { .. })
        ));
    }

    #[test]
    fn test_match_arm_calls() {
        let code = r#"
            fn handle_left() { unit }
            fn handle_right() { unit }

            fn main() {
                let s: Either<u1, u1> = Left(0);
                match s {
                    Left(l: u1) => handle_left(),
                    Right(r: u1) => handle_right(),
                }
            }
        "#;
        assert!(check_code(code).is_ok());
    }

    #[test]
    fn test_fail_match_arm_forward_ref() {
        let code = r#"
            fn main() {
                let s: Either<u1, u1> = Left(0);
                match s {
                    Left(l: u1) => unit,
                    Right(r: u1) => not_defined_yet(),
                }
            }

            fn not_defined_yet() { unit }
        "#;

        let result = check_code(code);
        match result {
            Err(OrderError::ForwardReference { used, .. }) => {
                assert_eq!(used.as_ref(), "not_defined_yet");
            }
            _ => panic!("Should catch forward ref in match arm, got: {:?}", result),
        }
    }

    #[test]
    fn test_multiple_failures_stop_at_first() {
        let code = r#"
            fn a() { missing1() }
            fn b() { missing2() }
        "#;

        match check_code(code) {
            Err(OrderError::ForwardReference { used, .. }) => {
                assert_eq!(used.as_ref(), "missing1");
            }
            _ => panic!("Should stop at first error"),
        }
    }
}