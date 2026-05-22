use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    Read,
    Create,
    Mutate,
    Delete,
    Admin,
}

impl fmt::Display for Tier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Tier::Read => write!(f, "read"),
            Tier::Create => write!(f, "create"),
            Tier::Mutate => write!(f, "mutate"),
            Tier::Delete => write!(f, "delete"),
            Tier::Admin => write!(f, "admin"),
        }
    }
}

fn keyword_tier(keyword: &str) -> Option<Tier> {
    match keyword {
        "SELECT" | "INFO" | "SHOW" | "LET" | "RETURN" => Some(Tier::Read),
        "CREATE" | "INSERT" | "RELATE" => Some(Tier::Create),
        "UPDATE" | "UPSERT" => Some(Tier::Mutate),
        "DELETE" | "REMOVE" => Some(Tier::Delete),
        "DEFINE" | "ALTER" | "REBUILD" | "ACCESS" | "USE" | "LIVE" | "KILL" | "BEGIN"
        | "COMMIT" | "CANCEL" | "FOR" | "CONTINUE" | "BREAK" | "IF" | "SLEEP" | "THROW" => {
            Some(Tier::Admin)
        }
        _ => None,
    }
}

/// Strip string literals, backtick-quoted identifiers, and comments from SQL,
/// replacing them with spaces so that statement-boundary offsets are preserved.
fn strip_strings_and_comments(sql: &str) -> String {
    let mut result = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            // Single-quoted or double-quoted strings
            '\'' | '"' => {
                result.push(' ');
                while let Some(inner) = chars.next() {
                    if inner == '\\' {
                        chars.next(); // skip escaped char
                    } else if inner == c {
                        break;
                    }
                }
            }
            // Backtick-quoted identifiers
            '`' => {
                result.push(' ');
                while let Some(inner) = chars.next() {
                    if inner == '`' {
                        break;
                    }
                }
            }
            // Line comments
            '-' if chars.peek() == Some(&'-') => {
                chars.next();
                result.push(' ');
                for inner in chars.by_ref() {
                    if inner == '\n' {
                        break;
                    }
                }
            }
            // Block comments
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                result.push(' ');
                while let Some(inner) = chars.next() {
                    if inner == '*' && chars.peek() == Some(&'/') {
                        chars.next();
                        break;
                    }
                }
            }
            _ => result.push(c),
        }
    }

    result
}

/// Classify a SurrealQL query by scanning for statement keywords at statement
/// boundaries (start of input, after `;`, after `(`).
///
/// Returns the maximum tier required and the keywords that were found.
pub fn classify_query(sql: &str) -> (Tier, Vec<String>) {
    let cleaned = strip_strings_and_comments(sql);
    let mut max_tier = Tier::Read;
    let mut found_keywords = Vec::new();

    // Split on statement boundaries: semicolons and open-parens (subqueries)
    for segment in cleaned.split(|c: char| c == ';' || c == '(') {
        let trimmed = segment.trim();
        if let Some(first_word) = trimmed.split_whitespace().next() {
            let upper = first_word.to_uppercase();
            if let Some(tier) = keyword_tier(&upper) {
                if tier > max_tier {
                    max_tier = tier;
                }
                found_keywords.push(upper);
            }
        }
    }

    (max_tier, found_keywords)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_select() {
        let (tier, _) = classify_query("SELECT * FROM person");
        assert_eq!(tier, Tier::Read);
    }

    #[test]
    fn select_with_let() {
        let (tier, _) = classify_query("LET $x = SELECT * FROM person; RETURN $x;");
        assert_eq!(tier, Tier::Read);
    }

    #[test]
    fn create_statement() {
        let (tier, _) = classify_query("CREATE person SET name = 'John'");
        assert_eq!(tier, Tier::Create);
    }

    #[test]
    fn update_statement() {
        let (tier, _) = classify_query("UPDATE person SET age = 30");
        assert_eq!(tier, Tier::Mutate);
    }

    #[test]
    fn delete_statement() {
        let (tier, _) = classify_query("DELETE person WHERE age < 18");
        assert_eq!(tier, Tier::Delete);
    }

    #[test]
    fn nested_delete_in_select() {
        let (tier, keywords) = classify_query("SELECT * FROM (DELETE person RETURN BEFORE)");
        assert_eq!(tier, Tier::Delete);
        assert!(keywords.contains(&"DELETE".to_string()));
    }

    #[test]
    fn delete_keyword_in_string_is_ignored() {
        let (tier, _) = classify_query("SELECT * FROM person WHERE name = 'DELETE me'");
        assert_eq!(tier, Tier::Read);
    }

    #[test]
    fn backtick_identifier_is_ignored() {
        let (tier, _) = classify_query("SELECT `delete` FROM person");
        assert_eq!(tier, Tier::Read);
    }

    #[test]
    fn multi_statement_max_tier() {
        let (tier, _) = classify_query("SELECT * FROM person; UPDATE person SET x = 1;");
        assert_eq!(tier, Tier::Mutate);
    }

    #[test]
    fn define_is_admin() {
        let (tier, _) = classify_query("DEFINE TABLE person SCHEMALESS");
        assert_eq!(tier, Tier::Admin);
    }

    #[test]
    fn relate_is_create() {
        let (tier, _) = classify_query("RELATE person:1->knows->person:2");
        assert_eq!(tier, Tier::Create);
    }

    #[test]
    fn comment_keyword_ignored() {
        let (tier, _) = classify_query("-- DELETE everything\nSELECT * FROM person");
        assert_eq!(tier, Tier::Read);
    }

    #[test]
    fn block_comment_keyword_ignored() {
        let (tier, _) = classify_query("/* DELETE */ SELECT * FROM person");
        assert_eq!(tier, Tier::Read);
    }
}
