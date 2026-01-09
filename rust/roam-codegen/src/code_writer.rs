use std::fmt;

/// A code writer that tracks indentation and provides helpers for generating
/// C-like syntax (used by Swift, TypeScript, Go, etc.)
pub struct CodeWriter<W> {
    writer: W,
    indent_level: usize,
    indent_string: String,
    at_line_start: bool,
}

impl<W: fmt::Write> CodeWriter<W> {
    /// Create a new CodeWriter with the given writer and indent string (e.g., "    " or "\t")
    pub fn new(writer: W, indent_string: String) -> Self {
        Self {
            writer,
            indent_level: 0,
            indent_string,
            at_line_start: true,
        }
    }

    /// Create a new CodeWriter with 4-space indentation
    pub fn with_indent_spaces(writer: W, spaces: usize) -> Self {
        Self::new(writer, " ".repeat(spaces))
    }

    /// Write text without a newline. Adds indentation if at line start.
    pub fn write(&mut self, text: &str) -> fmt::Result {
        if text.is_empty() {
            return Ok(());
        }

        if self.at_line_start && !text.trim().is_empty() {
            for _ in 0..self.indent_level {
                self.writer.write_str(&self.indent_string)?;
            }
            self.at_line_start = false;
        }

        self.writer.write_str(text)
    }

    /// Write text followed by a newline. Adds indentation if needed.
    pub fn writeln(&mut self, text: &str) -> fmt::Result {
        self.write(text)?;
        self.writer.write_char('\n')?;
        self.at_line_start = true;
        Ok(())
    }

    /// Write an empty line
    pub fn blank_line(&mut self) -> fmt::Result {
        self.writer.write_char('\n')?;
        self.at_line_start = true;
        Ok(())
    }

    /// Create an indentation guard. Indentation increases while the guard is alive.
    pub fn indent(&mut self) -> IndentGuard<'_, W> {
        self.indent_level += 1;
        IndentGuard { writer: self }
    }

    /// Write a single-line comment (e.g., "// comment")
    pub fn comment(&mut self, comment_prefix: &str, text: &str) -> fmt::Result {
        self.writeln(&format!("{} {}", comment_prefix, text))
    }

    /// Write a doc comment block. Each line is prefixed with the comment marker.
    pub fn doc_comment(&mut self, comment_prefix: &str, text: &str) -> fmt::Result {
        for line in text.lines() {
            self.writeln(&format!("{} {}", comment_prefix, line))?;
        }
        Ok(())
    }

    /// Begin a block with opening brace: writes "header {" and returns indent guard
    pub fn begin_block(&mut self, header: &str) -> Result<IndentGuard<'_, W>, fmt::Error> {
        self.writeln(&format!("{} {{", header))?;
        Ok(self.indent())
    }

    /// End a block with closing brace
    pub fn end_block(&mut self) -> fmt::Result {
        self.writeln("}")
    }

    /// Write a complete block with a closure for the body
    pub fn block<F>(&mut self, header: &str, body: F) -> fmt::Result
    where
        F: FnOnce(&mut Self) -> fmt::Result,
    {
        self.writeln(&format!("{} {{", header))?;
        {
            let _indent = self.indent();
            body(self)?;
        }
        self.writeln("}")
    }

    /// Get the current indentation level
    pub fn indent_level(&self) -> usize {
        self.indent_level
    }

    /// Consume the writer and return the inner writer
    pub fn into_inner(self) -> W {
        self.writer
    }

    /// Get a reference to the inner writer
    pub fn inner(&self) -> &W {
        &self.writer
    }

    /// Get a mutable reference to the inner writer
    pub fn inner_mut(&mut self) -> &mut W {
        &mut self.writer
    }
}

/// RAII guard that maintains indentation level
pub struct IndentGuard<'a, W> {
    writer: &'a mut CodeWriter<W>,
}

impl<W> Drop for IndentGuard<'_, W> {
    fn drop(&mut self) {
        self.writer.indent_level = self.writer.indent_level.saturating_sub(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_writing() {
        let mut output = String::new();
        let mut w = CodeWriter::with_indent_spaces(&mut output, 2);

        w.writeln("hello").unwrap();
        w.writeln("world").unwrap();

        assert_eq!(output, "hello\nworld\n");
    }

    #[test]
    fn test_indentation() {
        let mut output = String::new();
        let mut w = CodeWriter::with_indent_spaces(&mut output, 2);

        w.writeln("level 0").unwrap();
        {
            let _indent = w.indent();
            w.writeln("level 1").unwrap();
            {
                let _indent = w.indent();
                w.writeln("level 2").unwrap();
            }
            w.writeln("level 1 again").unwrap();
        }
        w.writeln("level 0 again").unwrap();

        assert_eq!(
            output,
            "level 0\n  level 1\n    level 2\n  level 1 again\nlevel 0 again\n"
        );
    }

    #[test]
    fn test_block_helper() {
        let mut output = String::new();
        let mut w = CodeWriter::with_indent_spaces(&mut output, 2);

        w.block("class Foo", |w| {
            w.writeln("let x = 42")?;
            w.block("func bar()", |w| w.writeln("return x"))
        })
        .unwrap();

        assert_eq!(
            output,
            "class Foo {\n  let x = 42\n  func bar() {\n    return x\n  }\n}\n"
        );
    }

    #[test]
    fn test_begin_end_block() {
        let mut output = String::new();
        let mut w = CodeWriter::with_indent_spaces(&mut output, 2);

        w.writeln("before").unwrap();
        {
            let _guard = w.begin_block("if true").unwrap();
            w.writeln("inside").unwrap();
        }
        w.end_block().unwrap();
        w.writeln("after").unwrap();

        assert_eq!(output, "before\nif true {\n  inside\n}\nafter\n");
    }

    #[test]
    fn test_comments() {
        let mut output = String::new();
        let mut w = CodeWriter::with_indent_spaces(&mut output, 2);

        w.comment("//", "Single line comment").unwrap();
        w.doc_comment("///", "Doc comment\nwith multiple lines")
            .unwrap();

        assert_eq!(
            output,
            "// Single line comment\n/// Doc comment\n/// with multiple lines\n"
        );
    }

    #[test]
    fn test_blank_lines() {
        let mut output = String::new();
        let mut w = CodeWriter::with_indent_spaces(&mut output, 2);

        w.writeln("line 1").unwrap();
        w.blank_line().unwrap();
        w.writeln("line 2").unwrap();

        assert_eq!(output, "line 1\n\nline 2\n");
    }
}
