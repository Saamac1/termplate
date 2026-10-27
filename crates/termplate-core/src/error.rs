//! Load errors. Every problem in a Termplate source is reported at load time
//! with the file and line where it was found (spec §1 "Virheet latauksessa").

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadError {
    pub file: String,
    pub line: usize,
    pub message: String,
}

impl LoadError {
    pub fn new(file: impl Into<String>, line: usize, message: impl Into<String>) -> Self {
        LoadError { file: file.into(), line, message: message.into() }
    }
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line == 0 {
            write!(f, "{}: {}", self.file, self.message)
        } else {
            write!(f, "{}:{}: {}", self.file, self.line, self.message)
        }
    }
}

impl std::error::Error for LoadError {}

/// A list of load errors; loading reports all problems it can find at once.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LoadErrors(pub Vec<LoadError>);

impl LoadErrors {
    pub fn push(&mut self, e: LoadError) {
        self.0.push(e);
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn into_result<T>(self, value: T) -> Result<T, LoadErrors> {
        if self.0.is_empty() { Ok(value) } else { Err(self) }
    }
}

impl From<LoadError> for LoadErrors {
    fn from(e: LoadError) -> Self {
        LoadErrors(vec![e])
    }
}

impl fmt::Display for LoadErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, e) in self.0.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "{e}")?;
        }
        Ok(())
    }
}

impl std::error::Error for LoadErrors {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_includes_file_and_line() {
        let e = LoadError::new("ui/main.termplate", 12, "unknown field `foo`");
        assert_eq!(e.to_string(), "ui/main.termplate:12: unknown field `foo`");
    }

    #[test]
    fn display_without_line() {
        let e = LoadError::new("app.termplate", 0, "missing app block");
        assert_eq!(e.to_string(), "app.termplate: missing app block");
    }

    #[test]
    fn errors_join_lines() {
        let mut errs = LoadErrors::default();
        errs.push(LoadError::new("a", 1, "x"));
        errs.push(LoadError::new("b", 2, "y"));
        assert_eq!(errs.to_string(), "a:1: x\nb:2: y");
        assert!(errs.into_result(()).is_err());
    }

    #[test]
    fn empty_errors_are_ok() {
        assert_eq!(LoadErrors::default().into_result(5), Ok(5));
    }
}
