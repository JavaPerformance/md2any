//! Tiny per-language tokenizer for code-block syntax highlighting.
//!
//! Not aiming for IDE-grade accuracy — the goal is "looks nice on a slide".
//! Each language has a small struct describing keywords, comment markers, and
//! string delimiters; a stateful tokenizer walks the source line-by-line and
//! produces colored token runs.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenKind {
    Default,
    Keyword,
    String,
    Number,
    Comment,
    Function,
    Type,
    Attribute,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub text: String,
    pub kind: TokenKind,
}

#[derive(Debug, Clone, Copy)]
struct Lang {
    line_comment: &'static [&'static str],
    block_comment: Option<(&'static str, &'static str)>,
    string_delims: &'static [char],
    triple_string: Option<&'static str>,
    keywords: &'static [&'static str],
    types: &'static [&'static str],
    constants: &'static [&'static str],
    capitalized_is_type: bool,
    attribute_starts: &'static [char],
    case_insensitive: bool,
    ident_extra: &'static str,
    col_comment_indicators: &'static [(usize, char)],
    line_full_comment_prefixes: &'static [&'static str],
}

const RUST_KEYWORDS: &[&str] = &[
    "as",
    "async",
    "await",
    "break",
    "const",
    "continue",
    "crate",
    "dyn",
    "else",
    "enum",
    "extern",
    "false",
    "fn",
    "for",
    "if",
    "impl",
    "in",
    "let",
    "loop",
    "match",
    "mod",
    "move",
    "mut",
    "pub",
    "ref",
    "return",
    "self",
    "Self",
    "static",
    "struct",
    "super",
    "trait",
    "true",
    "type",
    "unsafe",
    "use",
    "where",
    "while",
    "yield",
    "box",
    "macro_rules",
];
const RUST_TYPES: &[&str] = &[
    "bool", "char", "str", "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64",
    "u128", "usize", "f32", "f64", "String", "Vec", "Option", "Result", "Box", "Arc", "Rc",
    "HashMap", "BTreeMap", "HashSet", "BTreeSet",
];
const RUST_CONST: &[&str] = &["true", "false", "None", "Some", "Ok", "Err"];

const PYTHON_KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import",
    "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while",
    "with", "yield", "match", "case", "self",
];
const PYTHON_TYPES: &[&str] = &[
    "int",
    "float",
    "str",
    "bool",
    "list",
    "dict",
    "tuple",
    "set",
    "frozenset",
    "bytes",
    "bytearray",
    "type",
    "object",
    "Exception",
];
const PYTHON_CONST: &[&str] = &["True", "False", "None"];

const JS_KEYWORDS: &[&str] = &[
    "abstract",
    "as",
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "from",
    "function",
    "if",
    "implements",
    "import",
    "in",
    "instanceof",
    "interface",
    "is",
    "let",
    "module",
    "new",
    "null",
    "of",
    "package",
    "private",
    "protected",
    "public",
    "readonly",
    "return",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "type",
    "typeof",
    "undefined",
    "var",
    "void",
    "while",
    "with",
    "yield",
];
const JS_TYPES: &[&str] = &[
    "string", "number", "boolean", "any", "unknown", "never", "object", "void", "Array", "Promise",
    "Map", "Set", "Date", "RegExp",
];
const JS_CONST: &[&str] = &["true", "false", "null", "undefined"];

const GO_KEYWORDS: &[&str] = &[
    "break",
    "case",
    "chan",
    "const",
    "continue",
    "default",
    "defer",
    "else",
    "fallthrough",
    "for",
    "func",
    "go",
    "goto",
    "if",
    "import",
    "interface",
    "map",
    "package",
    "range",
    "return",
    "select",
    "struct",
    "switch",
    "type",
    "var",
];
const GO_TYPES: &[&str] = &[
    "bool",
    "byte",
    "complex64",
    "complex128",
    "error",
    "float32",
    "float64",
    "int",
    "int8",
    "int16",
    "int32",
    "int64",
    "rune",
    "string",
    "uint",
    "uint8",
    "uint16",
    "uint32",
    "uint64",
    "uintptr",
];
const GO_CONST: &[&str] = &["true", "false", "nil", "iota"];

const C_KEYWORDS: &[&str] = &[
    "auto",
    "break",
    "case",
    "char",
    "const",
    "continue",
    "default",
    "do",
    "double",
    "else",
    "enum",
    "extern",
    "float",
    "for",
    "goto",
    "if",
    "inline",
    "int",
    "long",
    "register",
    "restrict",
    "return",
    "short",
    "signed",
    "sizeof",
    "static",
    "struct",
    "switch",
    "typedef",
    "union",
    "unsigned",
    "void",
    "volatile",
    "while",
    "class",
    "namespace",
    "template",
    "typename",
    "public",
    "private",
    "protected",
    "virtual",
    "explicit",
    "operator",
    "new",
    "delete",
    "this",
    "true",
    "false",
    "nullptr",
    "throw",
    "try",
    "catch",
    "using",
    "constexpr",
    "noexcept",
    "auto",
];
const C_TYPES: &[&str] = &[
    "size_t",
    "ssize_t",
    "ptrdiff_t",
    "int8_t",
    "int16_t",
    "int32_t",
    "int64_t",
    "uint8_t",
    "uint16_t",
    "uint32_t",
    "uint64_t",
    "intptr_t",
    "uintptr_t",
    "FILE",
    "string",
    "vector",
    "map",
    "set",
    "unique_ptr",
    "shared_ptr",
];
const C_CONST: &[&str] = &["true", "false", "NULL", "nullptr"];

const JAVA_KEYWORDS: &[&str] = &[
    "abstract",
    "assert",
    "boolean",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "class",
    "const",
    "continue",
    "default",
    "do",
    "double",
    "else",
    "enum",
    "extends",
    "final",
    "finally",
    "float",
    "for",
    "goto",
    "if",
    "implements",
    "import",
    "instanceof",
    "int",
    "interface",
    "long",
    "native",
    "new",
    "package",
    "private",
    "protected",
    "public",
    "return",
    "short",
    "static",
    "strictfp",
    "super",
    "switch",
    "synchronized",
    "this",
    "throw",
    "throws",
    "transient",
    "try",
    "void",
    "volatile",
    "while",
    "true",
    "false",
    "null",
    "var",
];
const JAVA_TYPES: &[&str] = &[
    "String",
    "Object",
    "Integer",
    "Long",
    "Double",
    "Boolean",
    "Character",
    "List",
    "Map",
    "Set",
    "Optional",
];
const JAVA_CONST: &[&str] = &["true", "false", "null"];

const RUBY_KEYWORDS: &[&str] = &[
    "BEGIN",
    "END",
    "alias",
    "and",
    "begin",
    "break",
    "case",
    "class",
    "def",
    "defined?",
    "do",
    "else",
    "elsif",
    "end",
    "ensure",
    "false",
    "for",
    "if",
    "in",
    "module",
    "next",
    "nil",
    "not",
    "or",
    "redo",
    "rescue",
    "retry",
    "return",
    "self",
    "super",
    "then",
    "true",
    "undef",
    "unless",
    "until",
    "when",
    "while",
    "yield",
    "require",
    "require_relative",
    "attr_accessor",
    "attr_reader",
    "attr_writer",
];
const RUBY_TYPES: &[&str] = &[
    "Array", "Hash", "String", "Integer", "Float", "Symbol", "Proc", "Lambda",
];
const RUBY_CONST: &[&str] = &["true", "false", "nil"];

const BASH_KEYWORDS: &[&str] = &[
    "if", "then", "else", "elif", "fi", "case", "esac", "for", "while", "until", "do", "done",
    "in", "function", "return", "break", "continue", "exit", "export", "local", "readonly",
    "declare", "let", "select", "time", "set", "unset", "shift", "trap", "test", "true", "false",
    "source", "echo", "printf", "read", "cd", "pwd", "pushd", "popd",
];
const BASH_TYPES: &[&str] = &[];
const BASH_CONST: &[&str] = &["true", "false"];

const SQL_KEYWORDS: &[&str] = &[
    "SELECT",
    "FROM",
    "WHERE",
    "AND",
    "OR",
    "NOT",
    "IN",
    "EXISTS",
    "BETWEEN",
    "LIKE",
    "IS",
    "NULL",
    "ORDER",
    "BY",
    "GROUP",
    "HAVING",
    "LIMIT",
    "OFFSET",
    "JOIN",
    "INNER",
    "LEFT",
    "RIGHT",
    "FULL",
    "OUTER",
    "ON",
    "AS",
    "UNION",
    "ALL",
    "INSERT",
    "INTO",
    "VALUES",
    "UPDATE",
    "SET",
    "DELETE",
    "CREATE",
    "TABLE",
    "INDEX",
    "VIEW",
    "DROP",
    "ALTER",
    "ADD",
    "COLUMN",
    "PRIMARY",
    "KEY",
    "FOREIGN",
    "REFERENCES",
    "DEFAULT",
    "CHECK",
    "UNIQUE",
    "WITH",
    "CASE",
    "WHEN",
    "THEN",
    "ELSE",
    "END",
    "DISTINCT",
    "COUNT",
    "SUM",
    "AVG",
    "MIN",
    "MAX",
    "select",
    "from",
    "where",
    "and",
    "or",
    "not",
    "in",
    "exists",
    "between",
    "like",
    "is",
    "null",
    "order",
    "by",
    "group",
    "having",
    "limit",
    "offset",
    "join",
    "inner",
    "left",
    "right",
    "full",
    "outer",
    "on",
    "as",
    "union",
    "all",
    "insert",
    "into",
    "values",
    "update",
    "set",
    "delete",
    "create",
    "table",
    "index",
    "view",
    "drop",
    "alter",
    "add",
    "column",
    "primary",
    "key",
    "foreign",
    "references",
    "default",
    "check",
    "unique",
    "with",
    "case",
    "when",
    "then",
    "else",
    "end",
    "distinct",
    "count",
    "sum",
    "avg",
    "min",
    "max",
];
const SQL_TYPES: &[&str] = &[
    "INT",
    "INTEGER",
    "BIGINT",
    "SMALLINT",
    "VARCHAR",
    "CHAR",
    "TEXT",
    "DATE",
    "TIMESTAMP",
    "TIME",
    "BOOL",
    "BOOLEAN",
    "FLOAT",
    "DOUBLE",
    "DECIMAL",
    "NUMERIC",
    "JSON",
    "JSONB",
    "UUID",
    "BLOB",
    "BYTEA",
    "SERIAL",
    "int",
    "integer",
    "bigint",
    "smallint",
    "varchar",
    "char",
    "text",
    "date",
    "timestamp",
    "time",
    "bool",
    "boolean",
    "float",
    "double",
    "decimal",
    "numeric",
    "json",
    "jsonb",
    "uuid",
    "blob",
    "bytea",
    "serial",
];
const SQL_CONST: &[&str] = &["TRUE", "FALSE", "NULL", "true", "false", "null"];

const JSON_KEYWORDS: &[&str] = &[];
const JSON_TYPES: &[&str] = &[];
const JSON_CONST: &[&str] = &["true", "false", "null"];

const YAML_KEYWORDS: &[&str] = &[
    "true", "false", "null", "yes", "no", "on", "off", "True", "False", "Null", "Yes", "No",
];
const YAML_TYPES: &[&str] = &[];
const YAML_CONST: &[&str] = &[];

const TOML_KEYWORDS: &[&str] = &["true", "false"];
const TOML_TYPES: &[&str] = &[];
const TOML_CONST: &[&str] = &["true", "false"];

const HTML_KEYWORDS: &[&str] = &[];
const HTML_TYPES: &[&str] = &[];
const HTML_CONST: &[&str] = &[];

const CSS_KEYWORDS: &[&str] = &[
    "color",
    "background",
    "font",
    "margin",
    "padding",
    "border",
    "display",
    "flex",
    "grid",
    "position",
    "top",
    "left",
    "right",
    "bottom",
    "width",
    "height",
    "min",
    "max",
    "important",
    "inherit",
    "auto",
    "none",
    "block",
    "inline",
    "absolute",
    "relative",
    "fixed",
    "static",
    "sticky",
    "hidden",
    "visible",
    "transparent",
];
const CSS_TYPES: &[&str] = &[];
const CSS_CONST: &[&str] = &[];

// ---------------------------------------------------------------------------
// Additional developer / config languages
// ---------------------------------------------------------------------------

const HCL_KEYWORDS: &[&str] = &[
    "resource",
    "data",
    "module",
    "variable",
    "output",
    "locals",
    "provider",
    "terraform",
    "required_providers",
    "required_version",
    "backend",
    "dynamic",
    "for_each",
    "count",
    "depends_on",
    "lifecycle",
    "provisioner",
    "source",
    "version",
];
const HCL_TYPES: &[&str] = &[
    "string", "number", "bool", "list", "map", "object", "tuple", "set", "any",
];
const HCL_CONST: &[&str] = &["true", "false", "null"];

const DOCKERFILE_KEYWORDS: &[&str] = &[
    "FROM",
    "RUN",
    "CMD",
    "LABEL",
    "MAINTAINER",
    "EXPOSE",
    "ENV",
    "ADD",
    "COPY",
    "ENTRYPOINT",
    "VOLUME",
    "USER",
    "WORKDIR",
    "ARG",
    "ONBUILD",
    "STOPSIGNAL",
    "HEALTHCHECK",
    "SHELL",
    "AS",
];
const DOCKERFILE_TYPES: &[&str] = &[];
const DOCKERFILE_CONST: &[&str] = &["true", "false"];

const POWERSHELL_KEYWORDS: &[&str] = &[
    "begin",
    "break",
    "catch",
    "class",
    "continue",
    "data",
    "do",
    "dynamicparam",
    "else",
    "elseif",
    "end",
    "enum",
    "exit",
    "filter",
    "finally",
    "for",
    "foreach",
    "from",
    "function",
    "if",
    "in",
    "param",
    "process",
    "return",
    "switch",
    "throw",
    "trap",
    "try",
    "until",
    "using",
    "var",
    "while",
];
const POWERSHELL_TYPES: &[&str] = &[
    "string",
    "int",
    "long",
    "bool",
    "datetime",
    "array",
    "hashtable",
    "object",
    "pscustomobject",
];
const POWERSHELL_CONST: &[&str] = &[
    "$true",
    "$false",
    "$null",
    "$ErrorActionPreference",
    "$env",
    "true",
    "false",
    "null",
];

const PROPERTIES_KEYWORDS: &[&str] = &[];
const PROPERTIES_TYPES: &[&str] = &[];
const PROPERTIES_CONST: &[&str] = &[
    "true", "false", "yes", "no", "on", "off", "null", "enabled", "disabled",
];

const HASKELL_KEYWORDS: &[&str] = &[
    "case",
    "class",
    "data",
    "default",
    "deriving",
    "do",
    "else",
    "family",
    "forall",
    "foreign",
    "if",
    "import",
    "in",
    "infix",
    "infixl",
    "infixr",
    "instance",
    "let",
    "module",
    "newtype",
    "of",
    "qualified",
    "then",
    "type",
    "where",
];
const HASKELL_TYPES: &[&str] = &[
    "Bool", "Char", "Double", "Either", "Float", "IO", "Int", "Integer", "Maybe", "String",
];
const HASKELL_CONST: &[&str] = &["True", "False", "Nothing", "Just", "Left", "Right"];

const SCALA_KEYWORDS: &[&str] = &[
    "abstract",
    "case",
    "catch",
    "class",
    "def",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "false",
    "final",
    "finally",
    "for",
    "given",
    "if",
    "implicit",
    "import",
    "lazy",
    "match",
    "new",
    "null",
    "object",
    "override",
    "package",
    "private",
    "protected",
    "return",
    "sealed",
    "then",
    "this",
    "throw",
    "trait",
    "true",
    "try",
    "type",
    "val",
    "var",
    "while",
    "with",
    "yield",
];
const SCALA_TYPES: &[&str] = &[
    "Any", "AnyRef", "Boolean", "Byte", "Char", "Double", "Either", "Float", "Int", "List", "Long",
    "Map", "Option", "Seq", "Set", "Short", "String", "Unit",
];
const SCALA_CONST: &[&str] = &["true", "false", "null", "None", "Some", "Nil"];

const KOTLIN_KEYWORDS: &[&str] = &[
    "as",
    "break",
    "by",
    "catch",
    "class",
    "companion",
    "constructor",
    "continue",
    "data",
    "do",
    "else",
    "enum",
    "false",
    "finally",
    "for",
    "fun",
    "if",
    "import",
    "in",
    "inline",
    "interface",
    "is",
    "lateinit",
    "null",
    "object",
    "override",
    "package",
    "private",
    "protected",
    "public",
    "return",
    "sealed",
    "suspend",
    "this",
    "throw",
    "true",
    "try",
    "typealias",
    "val",
    "var",
    "when",
    "while",
];
const KOTLIN_TYPES: &[&str] = &[
    "Any",
    "Boolean",
    "Byte",
    "Char",
    "Double",
    "Float",
    "Int",
    "List",
    "Long",
    "Map",
    "MutableList",
    "MutableMap",
    "Nothing",
    "Sequence",
    "Set",
    "Short",
    "String",
    "Unit",
];
const KOTLIN_CONST: &[&str] = &["true", "false", "null"];

const CSHARP_KEYWORDS: &[&str] = &[
    "abstract",
    "as",
    "async",
    "await",
    "base",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "default",
    "delegate",
    "do",
    "else",
    "enum",
    "event",
    "explicit",
    "extern",
    "false",
    "finally",
    "fixed",
    "for",
    "foreach",
    "global",
    "if",
    "implicit",
    "in",
    "interface",
    "internal",
    "is",
    "lock",
    "namespace",
    "new",
    "null",
    "out",
    "override",
    "private",
    "protected",
    "public",
    "readonly",
    "record",
    "ref",
    "return",
    "sealed",
    "static",
    "struct",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "using",
    "virtual",
    "void",
    "while",
    "yield",
];
const CSHARP_TYPES: &[&str] = &[
    "bool",
    "byte",
    "char",
    "DateTime",
    "decimal",
    "double",
    "float",
    "Guid",
    "int",
    "IEnumerable",
    "List",
    "long",
    "object",
    "string",
    "Task",
    "var",
];
const CSHARP_CONST: &[&str] = &["true", "false", "null"];

const GRAPHQL_KEYWORDS: &[&str] = &[
    "directive",
    "enum",
    "extend",
    "fragment",
    "implements",
    "input",
    "interface",
    "mutation",
    "on",
    "query",
    "repeatable",
    "scalar",
    "schema",
    "subscription",
    "type",
    "union",
];
const GRAPHQL_TYPES: &[&str] = &["Boolean", "Float", "ID", "Int", "String"];
const GRAPHQL_CONST: &[&str] = &["true", "false", "null"];

const HTTP_KEYWORDS: &[&str] = &[
    "GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS", "CONNECT", "TRACE", "HTTP",
];
const HTTP_TYPES: &[&str] = &[
    "Accept",
    "Authorization",
    "Cache-Control",
    "Content-Length",
    "Content-Type",
    "Cookie",
    "Host",
    "Location",
    "Set-Cookie",
    "User-Agent",
];
const HTTP_CONST: &[&str] = &["true", "false", "null"];

const BCPL_KEYWORDS: &[&str] = &[
    "AND", "BE", "BREAK", "CASE", "DEFAULT", "DO", "ELSE", "FALSE", "FINISH", "FOR", "GLOBAL",
    "GOTO", "IF", "LET", "MANIFEST", "OR", "REPEAT", "RESULTIS", "RETURN", "STATIC", "SWITCHON",
    "TEST", "THEN", "TO", "TRUE", "UNLESS", "UNTIL", "VALOF", "VEC", "WHILE",
];
const BCPL_TYPES: &[&str] = &[];
const BCPL_CONST: &[&str] = &["TRUE", "FALSE"];

// ---------------------------------------------------------------------------
// Mainframe languages
// ---------------------------------------------------------------------------

const COBOL_KEYWORDS: &[&str] = &[
    "IDENTIFICATION",
    "DIVISION",
    "PROGRAM-ID",
    "AUTHOR",
    "INSTALLATION",
    "DATE-WRITTEN",
    "ENVIRONMENT",
    "CONFIGURATION",
    "SECTION",
    "SOURCE-COMPUTER",
    "OBJECT-COMPUTER",
    "INPUT-OUTPUT",
    "FILE-CONTROL",
    "I-O-CONTROL",
    "SELECT",
    "ASSIGN",
    "ORGANIZATION",
    "ACCESS",
    "MODE",
    "DATA",
    "FILE",
    "WORKING-STORAGE",
    "LINKAGE",
    "REPORT",
    "SCREEN",
    "PROCEDURE",
    "FD",
    "SD",
    "RD",
    "COPY",
    "INCLUDE",
    "REPLACING",
    "PIC",
    "PICTURE",
    "USAGE",
    "VALUE",
    "VALUES",
    "REDEFINES",
    "RENAMES",
    "OCCURS",
    "TIMES",
    "DEPENDING",
    "ON",
    "INDEXED",
    "KEY",
    "ASCENDING",
    "DESCENDING",
    "DISPLAY",
    "ACCEPT",
    "MOVE",
    "TO",
    "FROM",
    "ADD",
    "SUBTRACT",
    "MULTIPLY",
    "DIVIDE",
    "COMPUTE",
    "GIVING",
    "REMAINDER",
    "ROUNDED",
    "PERFORM",
    "VARYING",
    "UNTIL",
    "WHILE",
    "THRU",
    "THROUGH",
    "IF",
    "THEN",
    "ELSE",
    "END-IF",
    "EVALUATE",
    "WHEN",
    "OTHER",
    "END-EVALUATE",
    "EXIT",
    "STOP",
    "RUN",
    "GO",
    "GOTO",
    "CONTINUE",
    "NEXT",
    "SENTENCE",
    "CALL",
    "USING",
    "RETURNING",
    "READ",
    "WRITE",
    "REWRITE",
    "DELETE",
    "START",
    "OPEN",
    "CLOSE",
    "INPUT",
    "OUTPUT",
    "I-O",
    "EXTEND",
    "INVALID",
    "AT",
    "END",
    "STRING",
    "UNSTRING",
    "INSPECT",
    "TALLYING",
    "DELIMITED",
    "BY",
    "INTO",
    "INITIALIZE",
    "ALL",
    "SET",
    "SEARCH",
    "FILLER",
    "RECORD",
    "RECORDS",
    "BLOCK",
    "CONTAINS",
    "LABEL",
    "STANDARD",
    "OMITTED",
    "AND",
    "OR",
    "NOT",
    "IS",
    "ARE",
    "GREATER",
    "LESS",
    "EQUAL",
    "THAN",
    "AFTER",
    "BEFORE",
    "ADVANCING",
    "PAGE",
    "LINE",
    "LINES",
    "TOP",
    "BOTTOM",
    "EXEC",
    "END-EXEC",
];
const COBOL_TYPES: &[&str] = &[
    "COMP",
    "COMP-1",
    "COMP-2",
    "COMP-3",
    "COMP-4",
    "COMP-5",
    "BINARY",
    "PACKED-DECIMAL",
    "DISPLAY-1",
    "POINTER",
    "INDEX",
];
const COBOL_CONST: &[&str] = &[
    "SPACES",
    "SPACE",
    "ZEROS",
    "ZEROES",
    "ZERO",
    "HIGH-VALUES",
    "HIGH-VALUE",
    "LOW-VALUES",
    "LOW-VALUE",
    "QUOTES",
    "QUOTE",
    "NULL",
    "NULLS",
    "TRUE",
    "FALSE",
];

const LANG_COBOL: Lang = Lang {
    line_comment: &["*>"],
    block_comment: None,
    string_delims: &['"', '\''],
    triple_string: None,
    keywords: COBOL_KEYWORDS,
    types: COBOL_TYPES,
    constants: COBOL_CONST,
    capitalized_is_type: false,
    attribute_starts: &[],
    case_insensitive: true,
    ident_extra: "-",
    col_comment_indicators: &[(0, '*'), (6, '*'), (6, '/')],
    line_full_comment_prefixes: &[],
};

const JCL_KEYWORDS: &[&str] = &[
    "JOB",
    "EXEC",
    "DD",
    "PROC",
    "PEND",
    "INCLUDE",
    "JCLLIB",
    "OUTPUT",
    "OUTGROUP",
    "SET",
    "IF",
    "THEN",
    "ELSE",
    "ENDIF",
    "COMMAND",
    "DELIMITER",
    "EXPORT",
    "IMPORT",
    "CNTL",
    "ENDCNTL",
    "PGM",
    "DSN",
    "DSNAME",
    "DISP",
    "VOL",
    "VOLUME",
    "UNIT",
    "SPACE",
    "DCB",
    "RECFM",
    "LRECL",
    "BLKSIZE",
    "SYSOUT",
    "SYSIN",
    "STEPLIB",
    "JOBLIB",
    "JOBNAME",
    "CLASS",
    "MSGCLASS",
    "MSGLEVEL",
    "REGION",
    "TIME",
    "COND",
    "NOTIFY",
    "USER",
    "PARM",
    "RESTART",
    "TYPRUN",
    "ACCT",
    "ADDRSPC",
    "BYTES",
    "LINES",
    "PAGES",
];

const LANG_JCL: Lang = Lang {
    line_comment: &[],
    block_comment: None,
    string_delims: &['\''],
    triple_string: None,
    keywords: JCL_KEYWORDS,
    types: &[],
    constants: &[],
    capitalized_is_type: false,
    attribute_starts: &['&'],
    case_insensitive: true,
    ident_extra: "",
    col_comment_indicators: &[],
    line_full_comment_prefixes: &["//*"],
};

const REXX_KEYWORDS: &[&str] = &[
    "address",
    "arg",
    "by",
    "call",
    "do",
    "drop",
    "else",
    "end",
    "exit",
    "expose",
    "for",
    "forever",
    "if",
    "interpret",
    "iterate",
    "leave",
    "name",
    "nop",
    "numeric",
    "options",
    "otherwise",
    "parse",
    "procedure",
    "pull",
    "push",
    "queue",
    "return",
    "say",
    "select",
    "signal",
    "then",
    "to",
    "trace",
    "until",
    "upper",
    "value",
    "var",
    "when",
    "while",
    "with",
    "linein",
    "lineout",
    "stream",
    "charin",
    "charout",
    "external",
    "internal",
    "form",
    "digits",
    "fuzz",
    "scientific",
    "engineering",
    "halt",
    "novalue",
    "error",
    "failure",
    "notready",
    "syntax",
];

const LANG_REXX: Lang = Lang {
    line_comment: &[],
    block_comment: Some(("/*", "*/")),
    string_delims: &['"', '\''],
    triple_string: None,
    keywords: REXX_KEYWORDS,
    types: &[],
    constants: &[],
    capitalized_is_type: false,
    attribute_starts: &[],
    case_insensitive: true,
    ident_extra: "",
    col_comment_indicators: &[],
    line_full_comment_prefixes: &[],
};

const PLI_KEYWORDS: &[&str] = &[
    "PROCEDURE",
    "PROC",
    "BEGIN",
    "END",
    "DECLARE",
    "DCL",
    "DEFINE",
    "TYPE",
    "IF",
    "THEN",
    "ELSE",
    "DO",
    "WHILE",
    "UNTIL",
    "REPEAT",
    "TO",
    "BY",
    "GOTO",
    "GO",
    "RETURN",
    "RETURNS",
    "LEAVE",
    "ITERATE",
    "STOP",
    "EXIT",
    "SIGNAL",
    "REVERT",
    "ON",
    "OFF",
    "OPTIONS",
    "MAIN",
    "REENTRANT",
    "RECURSIVE",
    "REORDER",
    "ORDER",
    "INIT",
    "INITIAL",
    "STATIC",
    "AUTOMATIC",
    "AUTO",
    "BASED",
    "CONTROLLED",
    "CTL",
    "DEFINED",
    "DEF",
    "EXTERNAL",
    "EXT",
    "INTERNAL",
    "INT",
    "GLOBAL",
    "BUILTIN",
    "GENERIC",
    "ENTRY",
    "CALL",
    "ALLOCATE",
    "ALLOC",
    "FREE",
    "FETCH",
    "RELEASE",
    "OPEN",
    "CLOSE",
    "GET",
    "PUT",
    "READ",
    "WRITE",
    "REWRITE",
    "LOCATE",
    "DELETE",
    "SKIP",
    "LIST",
    "EDIT",
    "COLUMN",
    "COPY",
    "STRING",
    "FILE",
    "FROM",
    "INTO",
    "SELECT",
    "WHEN",
    "OTHERWISE",
    "INCLUDE",
    "PROCESS",
];
const PLI_TYPES: &[&str] = &[
    "FIXED",
    "FLOAT",
    "BIN",
    "BINARY",
    "DEC",
    "DECIMAL",
    "CHAR",
    "CHARACTER",
    "BIT",
    "PIC",
    "PICTURE",
    "POINTER",
    "PTR",
    "OFFSET",
    "AREA",
    "LABEL",
    "STRUCTURE",
    "STRUCT",
    "UNION",
    "ARRAY",
];
const PLI_CONST: &[&str] = &["NULL", "TRUE", "FALSE"];

const LANG_PLI: Lang = Lang {
    line_comment: &[],
    block_comment: Some(("/*", "*/")),
    string_delims: &['\''],
    triple_string: None,
    keywords: PLI_KEYWORDS,
    types: PLI_TYPES,
    constants: PLI_CONST,
    capitalized_is_type: false,
    attribute_starts: &['%'],
    case_insensitive: true,
    ident_extra: "_",
    col_comment_indicators: &[],
    line_full_comment_prefixes: &[],
};

const HLASM_KEYWORDS: &[&str] = &[
    "A", "AR", "AGR", "AH", "AHI", "AL", "ALR", "ALC", "ALCR", "ALG", "ALGR", "AP", "BAL", "BALR",
    "BAS", "BASR", "BC", "BCR", "BCT", "BCTR", "BE", "BER", "BH", "BL", "BM", "BNE", "BNH", "BNL",
    "BNM", "BNP", "BNZ", "BP", "BR", "BZ", "BRC", "BRCT", "BRAS", "C", "CL", "CLC", "CLI", "CLM",
    "CLR", "CLG", "CLGR", "CR", "CGR", "CH", "CHI", "D", "DR", "DP", "DLR", "DLGR", "ED", "EDMK",
    "EX", "EXRL", "IC", "ICM", "J", "JE", "JH", "JL", "JM", "JNE", "JNH", "JNL", "JNM", "JNZ",
    "JP", "JZ", "L", "LA", "LAE", "LAY", "LCR", "LD", "LE", "LG", "LGR", "LH", "LHI", "LM", "LMG",
    "LNR", "LPR", "LR", "LT", "LTR", "LARL", "M", "MH", "MHI", "ML", "MLR", "MP", "MR", "MS",
    "MSR", "MVC", "MVCL", "MVI", "MVCIN", "MVN", "MVZ", "N", "NC", "NI", "NR", "NG", "NGR", "O",
    "OC", "OI", "OR", "OG", "OGR", "PACK", "POPCNT", "S", "SH", "SL", "SLR", "SLA", "SLDA", "SLDL",
    "SLL", "SP", "SR", "SRA", "SRDA", "SRDL", "SRL", "ST", "STC", "STCM", "STD", "STE", "STG",
    "STH", "STM", "STMG", "STN", "TM", "TR", "TRT", "UNPK", "X", "XC", "XI", "XR", "XG", "XGR",
    "ZAP", "CSECT", "DSECT", "RSECT", "START", "END", "ENTRY", "EXTRN", "WXTRN", "USING", "DROP",
    "LTORG", "EQU", "DC", "DS", "DCB", "DCBD", "ORG", "PRINT", "SPACE", "TITLE", "EJECT",
    "INCLUDE", "COPY", "MACRO", "MEND", "MEXIT", "AIF", "AGO", "ANOP", "ASPACE", "GBLA", "GBLB",
    "GBLC", "LCLA", "LCLB", "LCLC", "SETA", "SETB", "SETC", "ACTR", "AREAD", "MNOTE", "PUSH",
    "POP", "ALIAS", "AMODE", "RMODE", "WTO", "ABEND", "SVC", "STIMER", "STIMERM", "TIME",
    "GETMAIN", "FREEMAIN", "OPEN", "CLOSE", "READ", "WRITE", "GET", "PUT", "BLDL", "LOAD", "LINK",
    "XCTL", "RETURN", "SAVE",
];

const LANG_HLASM: Lang = Lang {
    line_comment: &[],
    block_comment: None,
    string_delims: &['\''],
    triple_string: None,
    keywords: HLASM_KEYWORDS,
    types: &[],
    constants: &[],
    capitalized_is_type: false,
    attribute_starts: &['&'],
    case_insensitive: true,
    ident_extra: "@#$",
    col_comment_indicators: &[(0, '*')],
    line_full_comment_prefixes: &[".*"],
};

const DB2_KEYWORDS: &[&str] = &[
    "select",
    "from",
    "where",
    "and",
    "or",
    "not",
    "in",
    "exists",
    "between",
    "like",
    "is",
    "null",
    "order",
    "by",
    "group",
    "having",
    "limit",
    "offset",
    "fetch",
    "first",
    "rows",
    "only",
    "with",
    "ur",
    "cs",
    "rs",
    "rr",
    "join",
    "inner",
    "left",
    "right",
    "full",
    "outer",
    "cross",
    "on",
    "as",
    "using",
    "union",
    "intersect",
    "except",
    "all",
    "distinct",
    "insert",
    "into",
    "values",
    "update",
    "set",
    "delete",
    "create",
    "table",
    "tablespace",
    "index",
    "view",
    "alias",
    "synonym",
    "trigger",
    "function",
    "procedure",
    "package",
    "sequence",
    "schema",
    "database",
    "stogroup",
    "drop",
    "alter",
    "rename",
    "add",
    "column",
    "modify",
    "primary",
    "key",
    "foreign",
    "references",
    "default",
    "check",
    "unique",
    "constraint",
    "case",
    "when",
    "then",
    "else",
    "end",
    "count",
    "sum",
    "avg",
    "min",
    "max",
    "coalesce",
    "nullif",
    "cast",
    "convert",
    "current",
    "user",
    "session_user",
    "bufferpool",
    "indexbp",
    "locksize",
    "lockmax",
    "close",
    "vcat",
    "priqty",
    "secqty",
    "erase",
    "freepage",
    "pctfree",
    "compress",
    "trackmod",
    "logged",
    "member",
    "cluster",
    "segsize",
    "numparts",
    "dssize",
    "editproc",
    "validproc",
    "audit",
    "obid",
    "ccsid",
    "partition",
    "identity",
    "generated",
    "always",
    "cycle",
    "maxvalue",
    "minvalue",
    "no",
    "commit",
    "rollback",
    "savepoint",
    "begin",
    "atomic",
    "compound",
    "declare",
    "for",
    "do",
    "while",
    "repeat",
    "leave",
    "iterate",
    "return",
    "signal",
    "resignal",
    "if",
    "elseif",
    "loop",
    "row",
    "result_set",
    "language",
    "sql",
    "parameter",
    "style",
    "deterministic",
    "specific",
    "external",
    "name",
    "fenced",
    "label",
    "comment",
    "grant",
    "revoke",
    "to",
    "privileges",
];
const DB2_TYPES: &[&str] = &[
    "int",
    "integer",
    "bigint",
    "smallint",
    "decimal",
    "numeric",
    "decfloat",
    "real",
    "double",
    "float",
    "char",
    "character",
    "varchar",
    "graphic",
    "vargraphic",
    "clob",
    "dbclob",
    "blob",
    "binary",
    "varbinary",
    "date",
    "time",
    "timestamp",
    "rowid",
    "xml",
    "boolean",
];
const DB2_CONST: &[&str] = &[
    "true",
    "false",
    "null",
    "current_timestamp",
    "current_date",
    "current_time",
];

const LANG_DB2: Lang = Lang {
    line_comment: &["--"],
    block_comment: Some(("/*", "*/")),
    string_delims: &['\''],
    triple_string: None,
    keywords: DB2_KEYWORDS,
    types: DB2_TYPES,
    constants: DB2_CONST,
    capitalized_is_type: false,
    attribute_starts: &[],
    case_insensitive: true,
    ident_extra: "",
    col_comment_indicators: &[],
    line_full_comment_prefixes: &[],
};

const RPG_FIXED_KEYWORDS: &[&str] = &[
    "H", "F", "D", "I", "C", "O", "P", "ADD", "SUB", "MULT", "DIV", "MVR", "SQRT", "XFOOT",
    "Z-ADD", "Z-SUB", "MOVE", "MOVEL", "CAT", "XLATE", "SCAN", "CHECK", "CHEKR", "COMP", "TESTN",
    "TESTZ", "LOOKUP", "SETLL", "SETGT", "READ", "READC", "READE", "READP", "READPE", "CHAIN",
    "WRITE", "UPDATE", "DELETE", "EXFMT", "OPEN", "CLOSE", "EXCPT", "DSPLY", "DUMP", "DEBUG",
    "GOTO", "TAG", "CAB", "CABEQ", "CABGE", "CABGT", "CABLE", "CABLT", "CABNE", "IF", "IFEQ",
    "IFNE", "IFLT", "IFLE", "IFGT", "IFGE", "ELSE", "ENDIF", "DO", "DOU", "DOW", "ENDDO", "LEAVE",
    "ITER", "SELECT", "WHEN", "OTHER", "ENDSL", "BEGSR", "ENDSR", "CAS", "CASEQ", "CASNE", "CASLT",
    "CASLE", "CASGT", "CASGE", "CALL", "CALLP", "PARM", "PLIST", "KLIST", "KFLD", "DEFINE", "DEFN",
    "EVAL", "CLEAR", "RESET", "SETON", "SETOFF", "RETURN",
];
const RPG_FIXED_TYPES: &[&str] = &[
    "A", "B", "C", "D", "F", "G", "I", "N", "P", "S", "T", "U", "Z",
];
const RPG_CONST: &[&str] = &[
    "**FREE",
    "*ALL",
    "*BLANK",
    "*BLANKS",
    "*END",
    "*ENTRY",
    "*FILE",
    "*HIVAL",
    "*IN",
    "*INLR",
    "*INZSR",
    "*ISO",
    "*LOVAL",
    "*MDY",
    "*NODEBUGIO",
    "*NO",
    "*NULL",
    "*OFF",
    "*OMIT",
    "*ON",
    "*PSSR",
    "*SRCSTMT",
    "*START",
    "*YMD",
    "*YES",
    "*ZERO",
    "*ZEROS",
];

const LANG_RPG_FIXED: Lang = Lang {
    line_comment: &[],
    block_comment: Some(("/*", "*/")),
    string_delims: &['\''],
    triple_string: None,
    keywords: RPG_FIXED_KEYWORDS,
    types: RPG_FIXED_TYPES,
    constants: RPG_CONST,
    capitalized_is_type: false,
    attribute_starts: &[],
    case_insensitive: true,
    ident_extra: "_-*",
    col_comment_indicators: &[(6, '*'), (6, '/')],
    line_full_comment_prefixes: &[],
};

const RPG_FREE_KEYWORDS: &[&str] = &[
    "**FREE",
    "ctl-opt",
    "dcl-c",
    "dcl-ds",
    "dcl-f",
    "dcl-pi",
    "dcl-pr",
    "dcl-proc",
    "dcl-s",
    "end-ds",
    "end-for",
    "end-pi",
    "end-pr",
    "end-proc",
    "begsr",
    "endsr",
    "if",
    "elseif",
    "else",
    "endif",
    "for",
    "to",
    "by",
    "dow",
    "dou",
    "enddo",
    "select",
    "when",
    "other",
    "endsl",
    "monitor",
    "on-error",
    "endmon",
    "chain",
    "read",
    "readc",
    "reade",
    "readp",
    "readpe",
    "write",
    "update",
    "delete",
    "exfmt",
    "open",
    "close",
    "setll",
    "setgt",
    "eval",
    "callp",
    "return",
    "dsply",
    "clear",
    "reset",
    "leave",
    "iter",
    "in",
    "inz",
    "like",
    "likeds",
    "extname",
    "extpgm",
    "export",
    "import",
    "qualified",
    "template",
    "options",
    "const",
    "value",
    "varying",
];
const RPG_FREE_TYPES: &[&str] = &[
    "char",
    "varchar",
    "graph",
    "ucs2",
    "ind",
    "int",
    "uns",
    "packed",
    "zoned",
    "float",
    "date",
    "time",
    "timestamp",
    "pointer",
    "sqltype",
];

const LANG_RPG_FREE: Lang = Lang {
    line_comment: &["//"],
    block_comment: Some(("/*", "*/")),
    string_delims: &['\''],
    triple_string: None,
    keywords: RPG_FREE_KEYWORDS,
    types: RPG_FREE_TYPES,
    constants: RPG_CONST,
    capitalized_is_type: false,
    attribute_starts: &['%'],
    case_insensitive: true,
    ident_extra: "_-*",
    col_comment_indicators: &[],
    line_full_comment_prefixes: &[],
};

const CL_KEYWORDS: &[&str] = &[
    "PGM",
    "ENDPGM",
    "DCL",
    "DCLF",
    "CHGVAR",
    "IF",
    "THEN",
    "ELSE",
    "DO",
    "ENDDO",
    "DOFOR",
    "DOWHILE",
    "DOUNTIL",
    "SELECT",
    "WHEN",
    "OTHERWISE",
    "ENDSELECT",
    "MONMSG",
    "CALL",
    "CALLPRC",
    "RETURN",
    "GOTO",
    "SNDPGMMSG",
    "RCVMSG",
    "SNDBRKMSG",
    "SNDUSRMSG",
    "SBMJOB",
    "DLYJOB",
    "RTVJOBA",
    "RTVOBJD",
    "RTVDTAARA",
    "CHGDTAARA",
    "CRTDTAARA",
    "DLTDTAARA",
    "OVRDBF",
    "DLTOVR",
    "OPNQRYF",
    "CPYF",
    "CLRPFM",
    "RUNQRY",
    "CRTLIB",
    "DLTLIB",
    "ADDLIBLE",
    "RMVLIBLE",
    "CHGLIBL",
    "CHKOBJ",
    "CRTPF",
    "DLTF",
    "DSPFD",
    "DSPFFD",
    "WRKOBJ",
    "WRKACTJOB",
    "WRKSPLF",
    "STRCMTCTL",
    "COMMIT",
    "ROLLBACK",
    "CMDLBL",
    "PARM",
    "VAR",
    "TYPE",
    "LEN",
    "VALUE",
    "COND",
    "EXEC",
    "MSG",
    "MSGID",
    "MSGF",
    "MSGTYPE",
];
const CL_CONST: &[&str] = &[
    "*ALL", "*BLANK", "*CAT", "*BCAT", "*TCAT", "*CHAR", "*DEC", "*LGL", "*INT", "*UINT", "*YES",
    "*NO", "*ON", "*OFF", "*NONE", "*N", "*SAME", "*LIBL", "*CURLIB", "*JOB", "*PGM", "*ESCAPE",
    "*INFO", "*STATUS", "*COMP", "*DIAG", "*EQ", "*NE", "*GT", "*GE", "*LT", "*LE", "*AND", "*OR",
    "*NOT",
];

const LANG_CL: Lang = Lang {
    line_comment: &[],
    block_comment: Some(("/*", "*/")),
    string_delims: &['\''],
    triple_string: None,
    keywords: CL_KEYWORDS,
    types: &[],
    constants: CL_CONST,
    capitalized_is_type: false,
    attribute_starts: &['&'],
    case_insensitive: true,
    ident_extra: "_*#@$",
    col_comment_indicators: &[],
    line_full_comment_prefixes: &[],
};

fn get_lang(name: &str) -> Option<&'static Lang> {
    let n = name.to_lowercase();
    match n.as_str() {
        "rust" | "rs" => Some(&LANG_RUST),
        "python" | "py" | "python3" => Some(&LANG_PYTHON),
        "javascript" | "js" | "node" | "typescript" | "ts" | "tsx" | "jsx" => Some(&LANG_JS),
        "go" | "golang" => Some(&LANG_GO),
        "c" | "h" => Some(&LANG_C),
        "cpp" | "c++" | "cxx" | "hpp" | "cc" => Some(&LANG_CPP),
        "java" => Some(&LANG_JAVA),
        "kotlin" | "kt" | "kts" => Some(&LANG_KOTLIN),
        "scala" | "sc" => Some(&LANG_SCALA),
        "csharp" | "cs" | "c#" | "dotnet" => Some(&LANG_CSHARP),
        "haskell" | "hs" | "lhs" => Some(&LANG_HASKELL),
        "bcpl" => Some(&LANG_BCPL),
        "ruby" | "rb" => Some(&LANG_RUBY),
        "powershell" | "ps" | "pwsh" | "ps1" => Some(&LANG_POWERSHELL),
        "bash" | "sh" | "shell" | "zsh" | "fish" => Some(&LANG_BASH),
        "sql" | "postgres" | "postgresql" | "mysql" | "sqlite" => Some(&LANG_SQL),
        "terraform" | "hcl" | "tf" | "tfvars" => Some(&LANG_HCL),
        "dockerfile" | "containerfile" | "docker" => Some(&LANG_DOCKERFILE),
        "properties" | "props" | "ini" | "cfg" | "conf" | "env" | "dotenv" => {
            Some(&LANG_PROPERTIES)
        }
        "graphql" | "gql" => Some(&LANG_GRAPHQL),
        "http" | "rest" | "request" | "requests" => Some(&LANG_HTTP),
        "json" => Some(&LANG_JSON),
        "yaml" | "yml" => Some(&LANG_YAML),
        "toml" => Some(&LANG_TOML),
        "html" | "xml" | "svg" | "vue" | "svelte" | "astro" => Some(&LANG_HTML),
        "css" | "scss" | "less" => Some(&LANG_CSS),
        "cobol" | "cob" | "cbl" => Some(&LANG_COBOL),
        "jcl" => Some(&LANG_JCL),
        "rexx" | "rex" => Some(&LANG_REXX),
        "pl1" | "pli" | "pl/i" | "pli390" | "plx" | "pl/x" => Some(&LANG_PLI),
        "hlasm" | "asm" | "asm370" | "asm390" | "s390asm" | "ibmasm" => Some(&LANG_HLASM),
        "db2" | "db2sql" | "db2ddl" => Some(&LANG_DB2),
        "rpg" | "rpg2" | "rpgii" | "rpg3" | "rpgiii" | "rpg400" => Some(&LANG_RPG_FIXED),
        "rpgle" | "rpg4" | "rpgiv" | "rpgfree" | "rpg-free" | "free-rpg" | "sqlrpgle" => {
            Some(&LANG_RPG_FREE)
        }
        "cl" | "clp" | "clle" | "ibmcl" | "ibmi-cl" | "control-language" => Some(&LANG_CL),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy)]
enum SpecialLang {
    Diff,
    Markdown,
    Bf,
}

fn get_special_lang(name: &str) -> Option<SpecialLang> {
    let n = name.to_lowercase();
    match n.as_str() {
        "diff" | "patch" | "udiff" => Some(SpecialLang::Diff),
        "markdown" | "md" | "mdown" => Some(SpecialLang::Markdown),
        "bf" => Some(SpecialLang::Bf),
        _ => None,
    }
}

const DEFAULTS: LangDefaults = LangDefaults {
    case_insensitive: false,
    ident_extra: "",
    col_comment_indicators: &[],
    line_full_comment_prefixes: &[],
};

#[derive(Debug, Clone, Copy)]
struct LangDefaults {
    case_insensitive: bool,
    ident_extra: &'static str,
    col_comment_indicators: &'static [(usize, char)],
    line_full_comment_prefixes: &'static [&'static str],
}

const LANG_RUST: Lang = Lang {
    line_comment: &["//"],
    block_comment: Some(("/*", "*/")),
    string_delims: &['"'],
    triple_string: None,
    keywords: RUST_KEYWORDS,
    types: RUST_TYPES,
    constants: RUST_CONST,
    capitalized_is_type: true,
    attribute_starts: &['#'],
    case_insensitive: DEFAULTS.case_insensitive,
    ident_extra: DEFAULTS.ident_extra,
    col_comment_indicators: DEFAULTS.col_comment_indicators,
    line_full_comment_prefixes: DEFAULTS.line_full_comment_prefixes,
};

const LANG_PYTHON: Lang = Lang {
    line_comment: &["#"],
    block_comment: None,
    string_delims: &['"', '\''],
    triple_string: Some("\"\"\""),
    keywords: PYTHON_KEYWORDS,
    types: PYTHON_TYPES,
    constants: PYTHON_CONST,
    capitalized_is_type: true,
    attribute_starts: &['@'],
    case_insensitive: DEFAULTS.case_insensitive,
    ident_extra: DEFAULTS.ident_extra,
    col_comment_indicators: DEFAULTS.col_comment_indicators,
    line_full_comment_prefixes: DEFAULTS.line_full_comment_prefixes,
};

const LANG_JS: Lang = Lang {
    line_comment: &["//"],
    block_comment: Some(("/*", "*/")),
    string_delims: &['"', '\'', '`'],
    triple_string: None,
    keywords: JS_KEYWORDS,
    types: JS_TYPES,
    constants: JS_CONST,
    capitalized_is_type: true,
    attribute_starts: &['@'],
    case_insensitive: DEFAULTS.case_insensitive,
    ident_extra: DEFAULTS.ident_extra,
    col_comment_indicators: DEFAULTS.col_comment_indicators,
    line_full_comment_prefixes: DEFAULTS.line_full_comment_prefixes,
};

const LANG_GO: Lang = Lang {
    line_comment: &["//"],
    block_comment: Some(("/*", "*/")),
    string_delims: &['"', '`'],
    triple_string: None,
    keywords: GO_KEYWORDS,
    types: GO_TYPES,
    constants: GO_CONST,
    capitalized_is_type: false,
    attribute_starts: &[],
    case_insensitive: DEFAULTS.case_insensitive,
    ident_extra: DEFAULTS.ident_extra,
    col_comment_indicators: DEFAULTS.col_comment_indicators,
    line_full_comment_prefixes: DEFAULTS.line_full_comment_prefixes,
};

const LANG_C: Lang = Lang {
    line_comment: &["//"],
    block_comment: Some(("/*", "*/")),
    string_delims: &['"'],
    triple_string: None,
    keywords: C_KEYWORDS,
    types: C_TYPES,
    constants: C_CONST,
    capitalized_is_type: false,
    attribute_starts: &['#'],
    case_insensitive: DEFAULTS.case_insensitive,
    ident_extra: DEFAULTS.ident_extra,
    col_comment_indicators: DEFAULTS.col_comment_indicators,
    line_full_comment_prefixes: DEFAULTS.line_full_comment_prefixes,
};

const LANG_CPP: Lang = LANG_C;

const LANG_JAVA: Lang = Lang {
    line_comment: &["//"],
    block_comment: Some(("/*", "*/")),
    string_delims: &['"'],
    triple_string: None,
    keywords: JAVA_KEYWORDS,
    types: JAVA_TYPES,
    constants: JAVA_CONST,
    capitalized_is_type: true,
    attribute_starts: &['@'],
    case_insensitive: DEFAULTS.case_insensitive,
    ident_extra: DEFAULTS.ident_extra,
    col_comment_indicators: DEFAULTS.col_comment_indicators,
    line_full_comment_prefixes: DEFAULTS.line_full_comment_prefixes,
};

const LANG_RUBY: Lang = Lang {
    line_comment: &["#"],
    block_comment: Some(("=begin", "=end")),
    string_delims: &['"', '\''],
    triple_string: None,
    keywords: RUBY_KEYWORDS,
    types: RUBY_TYPES,
    constants: RUBY_CONST,
    capitalized_is_type: true,
    attribute_starts: &['@', '$'],
    case_insensitive: DEFAULTS.case_insensitive,
    ident_extra: DEFAULTS.ident_extra,
    col_comment_indicators: DEFAULTS.col_comment_indicators,
    line_full_comment_prefixes: DEFAULTS.line_full_comment_prefixes,
};

const LANG_BASH: Lang = Lang {
    line_comment: &["#"],
    block_comment: None,
    string_delims: &['"', '\''],
    triple_string: None,
    keywords: BASH_KEYWORDS,
    types: BASH_TYPES,
    constants: BASH_CONST,
    capitalized_is_type: false,
    attribute_starts: &['$'],
    case_insensitive: DEFAULTS.case_insensitive,
    ident_extra: DEFAULTS.ident_extra,
    col_comment_indicators: DEFAULTS.col_comment_indicators,
    line_full_comment_prefixes: DEFAULTS.line_full_comment_prefixes,
};

const LANG_SQL: Lang = Lang {
    line_comment: &["--"],
    block_comment: Some(("/*", "*/")),
    string_delims: &['\''],
    triple_string: None,
    keywords: SQL_KEYWORDS,
    types: SQL_TYPES,
    constants: SQL_CONST,
    capitalized_is_type: false,
    attribute_starts: &[],
    case_insensitive: true,
    ident_extra: DEFAULTS.ident_extra,
    col_comment_indicators: DEFAULTS.col_comment_indicators,
    line_full_comment_prefixes: DEFAULTS.line_full_comment_prefixes,
};

const LANG_JSON: Lang = Lang {
    line_comment: &[],
    block_comment: None,
    string_delims: &['"'],
    triple_string: None,
    keywords: JSON_KEYWORDS,
    types: JSON_TYPES,
    constants: JSON_CONST,
    capitalized_is_type: false,
    attribute_starts: &[],
    case_insensitive: DEFAULTS.case_insensitive,
    ident_extra: DEFAULTS.ident_extra,
    col_comment_indicators: DEFAULTS.col_comment_indicators,
    line_full_comment_prefixes: DEFAULTS.line_full_comment_prefixes,
};

const LANG_YAML: Lang = Lang {
    line_comment: &["#"],
    block_comment: None,
    string_delims: &['"', '\''],
    triple_string: None,
    keywords: YAML_KEYWORDS,
    types: YAML_TYPES,
    constants: YAML_CONST,
    capitalized_is_type: false,
    attribute_starts: &[],
    case_insensitive: DEFAULTS.case_insensitive,
    ident_extra: DEFAULTS.ident_extra,
    col_comment_indicators: DEFAULTS.col_comment_indicators,
    line_full_comment_prefixes: DEFAULTS.line_full_comment_prefixes,
};

const LANG_TOML: Lang = Lang {
    line_comment: &["#"],
    block_comment: None,
    string_delims: &['"', '\''],
    triple_string: None,
    keywords: TOML_KEYWORDS,
    types: TOML_TYPES,
    constants: TOML_CONST,
    capitalized_is_type: false,
    attribute_starts: &[],
    case_insensitive: DEFAULTS.case_insensitive,
    ident_extra: DEFAULTS.ident_extra,
    col_comment_indicators: DEFAULTS.col_comment_indicators,
    line_full_comment_prefixes: DEFAULTS.line_full_comment_prefixes,
};

const LANG_HTML: Lang = Lang {
    line_comment: &[],
    block_comment: Some(("<!--", "-->")),
    string_delims: &['"', '\''],
    triple_string: None,
    keywords: HTML_KEYWORDS,
    types: HTML_TYPES,
    constants: HTML_CONST,
    capitalized_is_type: false,
    attribute_starts: &[],
    case_insensitive: DEFAULTS.case_insensitive,
    ident_extra: DEFAULTS.ident_extra,
    col_comment_indicators: DEFAULTS.col_comment_indicators,
    line_full_comment_prefixes: DEFAULTS.line_full_comment_prefixes,
};

const LANG_CSS: Lang = Lang {
    line_comment: &[],
    block_comment: Some(("/*", "*/")),
    string_delims: &['"', '\''],
    triple_string: None,
    keywords: CSS_KEYWORDS,
    types: CSS_TYPES,
    constants: CSS_CONST,
    capitalized_is_type: false,
    attribute_starts: &[],
    case_insensitive: DEFAULTS.case_insensitive,
    ident_extra: DEFAULTS.ident_extra,
    col_comment_indicators: DEFAULTS.col_comment_indicators,
    line_full_comment_prefixes: DEFAULTS.line_full_comment_prefixes,
};

const LANG_HCL: Lang = Lang {
    line_comment: &["#", "//"],
    block_comment: Some(("/*", "*/")),
    string_delims: &['"'],
    triple_string: None,
    keywords: HCL_KEYWORDS,
    types: HCL_TYPES,
    constants: HCL_CONST,
    capitalized_is_type: false,
    attribute_starts: &[],
    case_insensitive: DEFAULTS.case_insensitive,
    ident_extra: "-",
    col_comment_indicators: DEFAULTS.col_comment_indicators,
    line_full_comment_prefixes: DEFAULTS.line_full_comment_prefixes,
};

const LANG_DOCKERFILE: Lang = Lang {
    line_comment: &["#"],
    block_comment: None,
    string_delims: &['"', '\''],
    triple_string: None,
    keywords: DOCKERFILE_KEYWORDS,
    types: DOCKERFILE_TYPES,
    constants: DOCKERFILE_CONST,
    capitalized_is_type: false,
    attribute_starts: &[],
    case_insensitive: true,
    ident_extra: "_-",
    col_comment_indicators: DEFAULTS.col_comment_indicators,
    line_full_comment_prefixes: DEFAULTS.line_full_comment_prefixes,
};

const LANG_POWERSHELL: Lang = Lang {
    line_comment: &["#"],
    block_comment: Some(("<#", "#>")),
    string_delims: &['"', '\''],
    triple_string: None,
    keywords: POWERSHELL_KEYWORDS,
    types: POWERSHELL_TYPES,
    constants: POWERSHELL_CONST,
    capitalized_is_type: true,
    attribute_starts: &['$', '@'],
    case_insensitive: true,
    ident_extra: "-:$",
    col_comment_indicators: DEFAULTS.col_comment_indicators,
    line_full_comment_prefixes: DEFAULTS.line_full_comment_prefixes,
};

const LANG_PROPERTIES: Lang = Lang {
    line_comment: &["#", ";"],
    block_comment: None,
    string_delims: &['"', '\''],
    triple_string: None,
    keywords: PROPERTIES_KEYWORDS,
    types: PROPERTIES_TYPES,
    constants: PROPERTIES_CONST,
    capitalized_is_type: false,
    attribute_starts: &[],
    case_insensitive: true,
    ident_extra: "._-",
    col_comment_indicators: DEFAULTS.col_comment_indicators,
    line_full_comment_prefixes: DEFAULTS.line_full_comment_prefixes,
};

const LANG_HASKELL: Lang = Lang {
    line_comment: &["--"],
    block_comment: Some(("{-", "-}")),
    string_delims: &['"', '\''],
    triple_string: None,
    keywords: HASKELL_KEYWORDS,
    types: HASKELL_TYPES,
    constants: HASKELL_CONST,
    capitalized_is_type: true,
    attribute_starts: &[],
    case_insensitive: DEFAULTS.case_insensitive,
    ident_extra: "_'",
    col_comment_indicators: DEFAULTS.col_comment_indicators,
    line_full_comment_prefixes: DEFAULTS.line_full_comment_prefixes,
};

const LANG_SCALA: Lang = Lang {
    line_comment: &["//"],
    block_comment: Some(("/*", "*/")),
    string_delims: &['"', '\''],
    triple_string: Some("\"\"\""),
    keywords: SCALA_KEYWORDS,
    types: SCALA_TYPES,
    constants: SCALA_CONST,
    capitalized_is_type: true,
    attribute_starts: &['@'],
    case_insensitive: DEFAULTS.case_insensitive,
    ident_extra: "_",
    col_comment_indicators: DEFAULTS.col_comment_indicators,
    line_full_comment_prefixes: DEFAULTS.line_full_comment_prefixes,
};

const LANG_KOTLIN: Lang = Lang {
    line_comment: &["//"],
    block_comment: Some(("/*", "*/")),
    string_delims: &['"', '\''],
    triple_string: Some("\"\"\""),
    keywords: KOTLIN_KEYWORDS,
    types: KOTLIN_TYPES,
    constants: KOTLIN_CONST,
    capitalized_is_type: true,
    attribute_starts: &['@'],
    case_insensitive: DEFAULTS.case_insensitive,
    ident_extra: "_",
    col_comment_indicators: DEFAULTS.col_comment_indicators,
    line_full_comment_prefixes: DEFAULTS.line_full_comment_prefixes,
};

const LANG_CSHARP: Lang = Lang {
    line_comment: &["//"],
    block_comment: Some(("/*", "*/")),
    string_delims: &['"', '\''],
    triple_string: None,
    keywords: CSHARP_KEYWORDS,
    types: CSHARP_TYPES,
    constants: CSHARP_CONST,
    capitalized_is_type: true,
    attribute_starts: &['@', '['],
    case_insensitive: DEFAULTS.case_insensitive,
    ident_extra: "_",
    col_comment_indicators: DEFAULTS.col_comment_indicators,
    line_full_comment_prefixes: DEFAULTS.line_full_comment_prefixes,
};

const LANG_GRAPHQL: Lang = Lang {
    line_comment: &["#"],
    block_comment: None,
    string_delims: &['"'],
    triple_string: Some("\"\"\""),
    keywords: GRAPHQL_KEYWORDS,
    types: GRAPHQL_TYPES,
    constants: GRAPHQL_CONST,
    capitalized_is_type: true,
    attribute_starts: &['@', '$'],
    case_insensitive: DEFAULTS.case_insensitive,
    ident_extra: "_",
    col_comment_indicators: DEFAULTS.col_comment_indicators,
    line_full_comment_prefixes: DEFAULTS.line_full_comment_prefixes,
};

const LANG_HTTP: Lang = Lang {
    line_comment: &[],
    block_comment: None,
    string_delims: &['"'],
    triple_string: None,
    keywords: HTTP_KEYWORDS,
    types: HTTP_TYPES,
    constants: HTTP_CONST,
    capitalized_is_type: false,
    attribute_starts: &[],
    case_insensitive: false,
    ident_extra: "-/",
    col_comment_indicators: DEFAULTS.col_comment_indicators,
    line_full_comment_prefixes: DEFAULTS.line_full_comment_prefixes,
};

const LANG_BCPL: Lang = Lang {
    line_comment: &["//"],
    block_comment: Some(("/*", "*/")),
    string_delims: &['"', '\''],
    triple_string: None,
    keywords: BCPL_KEYWORDS,
    types: BCPL_TYPES,
    constants: BCPL_CONST,
    capitalized_is_type: false,
    attribute_starts: &[],
    case_insensitive: true,
    ident_extra: "_",
    col_comment_indicators: DEFAULTS.col_comment_indicators,
    line_full_comment_prefixes: DEFAULTS.line_full_comment_prefixes,
};

#[derive(Default, Clone, Copy)]
pub struct State {
    in_block_comment: bool,
    in_triple_string: bool,
}

pub fn tokenize(lines: &[String], lang_name: Option<&str>) -> Vec<Vec<Token>> {
    if let Some(special) = lang_name.and_then(get_special_lang) {
        return lines
            .iter()
            .map(|line| tokenize_special_line(line, special))
            .collect();
    }
    let Some(lang) = lang_name.and_then(get_lang) else {
        return lines
            .iter()
            .map(|l| {
                vec![Token {
                    text: l.clone(),
                    kind: TokenKind::Default,
                }]
            })
            .collect();
    };
    let mut state = State::default();
    lines
        .iter()
        .map(|line| tokenize_line(line, lang, &mut state))
        .collect()
}

fn tokenize_special_line(line: &str, special: SpecialLang) -> Vec<Token> {
    match special {
        SpecialLang::Diff => tokenize_diff_line(line),
        SpecialLang::Markdown => tokenize_markdown_line(line),
        SpecialLang::Bf => tokenize_bf_line(line),
    }
}

fn one_line(line: &str, kind: TokenKind) -> Vec<Token> {
    vec![Token {
        text: line.to_string(),
        kind,
    }]
}

fn tokenize_diff_line(line: &str) -> Vec<Token> {
    if line.starts_with("@@") {
        one_line(line, TokenKind::Keyword)
    } else if line.starts_with("diff ")
        || line.starts_with("index ")
        || line.starts_with("+++")
        || line.starts_with("---")
    {
        one_line(line, TokenKind::Comment)
    } else if line.starts_with('+') {
        one_line(line, TokenKind::String)
    } else if line.starts_with('-') {
        one_line(line, TokenKind::Attribute)
    } else {
        one_line(line, TokenKind::Default)
    }
}

fn tokenize_markdown_line(line: &str) -> Vec<Token> {
    let trimmed = line.trim_start();
    if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
        one_line(line, TokenKind::String)
    } else if trimmed.starts_with('#') {
        one_line(line, TokenKind::Keyword)
    } else if trimmed.starts_with('>') {
        one_line(line, TokenKind::Comment)
    } else if trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("+ ") {
        one_line(line, TokenKind::Attribute)
    } else {
        one_line(line, TokenKind::Default)
    }
}

fn tokenize_bf_line(line: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut buf = String::new();
    let mut command = String::new();
    for ch in line.chars() {
        if matches!(ch, '+' | '-' | '<' | '>' | '[' | ']' | '.' | ',') {
            flush(&mut tokens, &mut buf);
            command.push(ch);
        } else {
            if !command.is_empty() {
                tokens.push(Token {
                    text: std::mem::take(&mut command),
                    kind: TokenKind::Keyword,
                });
            }
            buf.push(ch);
        }
    }
    if !command.is_empty() {
        tokens.push(Token {
            text: command,
            kind: TokenKind::Keyword,
        });
    }
    flush(&mut tokens, &mut buf);
    tokens
}

fn tokenize_line(line: &str, lang: &Lang, state: &mut State) -> Vec<Token> {
    let chars: Vec<char> = line.chars().collect();
    let mut tokens: Vec<Token> = Vec::new();
    let mut buf = String::new();
    let mut i = 0;

    if !state.in_block_comment && !state.in_triple_string {
        for (col, marker) in lang.col_comment_indicators {
            if chars.get(*col) == Some(marker) {
                tokens.push(Token {
                    text: line.to_string(),
                    kind: TokenKind::Comment,
                });
                return tokens;
            }
        }
        for prefix in lang.line_full_comment_prefixes {
            if line.starts_with(prefix) {
                tokens.push(Token {
                    text: line.to_string(),
                    kind: TokenKind::Comment,
                });
                return tokens;
            }
        }
    }

    if state.in_block_comment {
        if let Some((_, end)) = lang.block_comment {
            let end_chars: Vec<char> = end.chars().collect();
            if let Some(pos) = find_substr(&chars, 0, &end_chars) {
                let text: String = chars[..pos + end_chars.len()].iter().collect();
                tokens.push(Token {
                    text,
                    kind: TokenKind::Comment,
                });
                i = pos + end_chars.len();
                state.in_block_comment = false;
            } else {
                tokens.push(Token {
                    text: line.to_string(),
                    kind: TokenKind::Comment,
                });
                return tokens;
            }
        }
    }

    if state.in_triple_string {
        if let Some(triple) = lang.triple_string {
            let tch: Vec<char> = triple.chars().collect();
            if let Some(pos) = find_substr(&chars, i, &tch) {
                let text: String = chars[i..pos + tch.len()].iter().collect();
                tokens.push(Token {
                    text,
                    kind: TokenKind::String,
                });
                i = pos + tch.len();
                state.in_triple_string = false;
            } else {
                let text: String = chars[i..].iter().collect();
                tokens.push(Token {
                    text,
                    kind: TokenKind::String,
                });
                return tokens;
            }
        }
    }

    while i < chars.len() {
        for lc in lang.line_comment {
            let lcc: Vec<char> = lc.chars().collect();
            if starts_with_at(&chars, i, &lcc) {
                flush(&mut tokens, &mut buf);
                let text: String = chars[i..].iter().collect();
                tokens.push(Token {
                    text,
                    kind: TokenKind::Comment,
                });
                return tokens;
            }
        }

        if let Some((start, end)) = lang.block_comment {
            let sc: Vec<char> = start.chars().collect();
            if starts_with_at(&chars, i, &sc) {
                flush(&mut tokens, &mut buf);
                let ec: Vec<char> = end.chars().collect();
                let search_from = i + sc.len();
                if let Some(pos) = find_substr(&chars, search_from, &ec) {
                    let text: String = chars[i..pos + ec.len()].iter().collect();
                    tokens.push(Token {
                        text,
                        kind: TokenKind::Comment,
                    });
                    i = pos + ec.len();
                } else {
                    let text: String = chars[i..].iter().collect();
                    tokens.push(Token {
                        text,
                        kind: TokenKind::Comment,
                    });
                    state.in_block_comment = true;
                    return tokens;
                }
                continue;
            }
        }

        if let Some(triple) = lang.triple_string {
            let tc: Vec<char> = triple.chars().collect();
            if starts_with_at(&chars, i, &tc) {
                flush(&mut tokens, &mut buf);
                let search_from = i + tc.len();
                if let Some(pos) = find_substr(&chars, search_from, &tc) {
                    let text: String = chars[i..pos + tc.len()].iter().collect();
                    tokens.push(Token {
                        text,
                        kind: TokenKind::String,
                    });
                    i = pos + tc.len();
                } else {
                    let text: String = chars[i..].iter().collect();
                    tokens.push(Token {
                        text,
                        kind: TokenKind::String,
                    });
                    state.in_triple_string = true;
                    return tokens;
                }
                continue;
            }
        }

        let c = chars[i];

        if lang.string_delims.contains(&c) {
            flush(&mut tokens, &mut buf);
            let mut text = String::new();
            text.push(c);
            i += 1;
            while i < chars.len() {
                let ch = chars[i];
                text.push(ch);
                i += 1;
                if ch == '\\' && i < chars.len() {
                    text.push(chars[i]);
                    i += 1;
                    continue;
                }
                if ch == c {
                    break;
                }
            }
            tokens.push(Token {
                text,
                kind: TokenKind::String,
            });
            continue;
        }

        if c.is_ascii_digit() {
            flush(&mut tokens, &mut buf);
            let start = i;
            i += 1;
            while i < chars.len() {
                let ch = chars[i];
                if ch.is_ascii_digit()
                    || ch == '.'
                    || ch == '_'
                    || matches!(ch, 'x' | 'X' | 'o' | 'O' | 'b' | 'B')
                    || matches!(ch, 'a'..='f' | 'A'..='F')
                    || matches!(ch, 'e' | 'E')
                    || ch == '+' && matches!(chars.get(i.wrapping_sub(1)), Some('e' | 'E'))
                    || ch == '-' && matches!(chars.get(i.wrapping_sub(1)), Some('e' | 'E'))
                {
                    i += 1;
                } else {
                    break;
                }
            }
            let text: String = chars[start..i].iter().collect();
            tokens.push(Token {
                text,
                kind: TokenKind::Number,
            });
            continue;
        }

        if c.is_alphabetic() || c == '_' || lang.ident_extra.contains(c) {
            flush(&mut tokens, &mut buf);
            let start = i;
            while i < chars.len()
                && (chars[i].is_alphanumeric()
                    || chars[i] == '_'
                    || lang.ident_extra.contains(chars[i]))
            {
                i += 1;
            }
            let ident: String = chars[start..i].iter().collect();

            let kind = if matches_list(&ident, lang.keywords, lang.case_insensitive)
                || matches_list(&ident, lang.constants, lang.case_insensitive)
            {
                TokenKind::Keyword
            } else if matches_list(&ident, lang.types, lang.case_insensitive) {
                TokenKind::Type
            } else if lang.capitalized_is_type
                && ident.chars().next().map_or(false, |c| c.is_uppercase())
                && ident.chars().any(|c| c.is_lowercase())
            {
                TokenKind::Type
            } else {
                let mut j = i;
                while j < chars.len() && chars[j] == ' ' {
                    j += 1;
                }
                if j < chars.len() && chars[j] == '(' {
                    TokenKind::Function
                } else {
                    TokenKind::Default
                }
            };
            tokens.push(Token { text: ident, kind });
            continue;
        }

        if lang.attribute_starts.contains(&c) {
            let next_is_ident = chars
                .get(i + 1)
                .map_or(false, |&n| n.is_alphabetic() || n == '_' || n == '[');
            if next_is_ident {
                flush(&mut tokens, &mut buf);
                let start = i;
                i += 1;
                if chars.get(i) == Some(&'[') {
                    while i < chars.len() && chars[i] != ']' {
                        i += 1;
                    }
                    if i < chars.len() {
                        i += 1;
                    }
                } else {
                    while i < chars.len()
                        && (chars[i].is_alphanumeric()
                            || chars[i] == '_'
                            || chars[i] == '!'
                            || chars[i] == ':')
                    {
                        i += 1;
                    }
                }
                let text: String = chars[start..i].iter().collect();
                tokens.push(Token {
                    text,
                    kind: TokenKind::Attribute,
                });
                continue;
            }
        }

        buf.push(c);
        i += 1;
    }

    flush(&mut tokens, &mut buf);
    tokens
}

fn matches_list(ident: &str, list: &[&str], case_insensitive: bool) -> bool {
    if case_insensitive {
        list.iter().any(|k| k.eq_ignore_ascii_case(ident))
    } else {
        list.iter().any(|k| *k == ident)
    }
}

fn flush(tokens: &mut Vec<Token>, buf: &mut String) {
    if buf.is_empty() {
        return;
    }
    let text = std::mem::take(buf);
    tokens.push(Token {
        text,
        kind: TokenKind::Default,
    });
}

fn starts_with_at(chars: &[char], at: usize, needle: &[char]) -> bool {
    if at + needle.len() > chars.len() {
        return false;
    }
    for (i, c) in needle.iter().enumerate() {
        if chars[at + i] != *c {
            return false;
        }
    }
    true
}

fn find_substr(chars: &[char], from: usize, needle: &[char]) -> Option<usize> {
    if needle.is_empty() || from + needle.len() > chars.len() {
        return None;
    }
    for i in from..=chars.len() - needle.len() {
        if starts_with_at(chars, i, needle) {
            return Some(i);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has_kind(lang: &str, line: &str, kind: TokenKind) -> bool {
        tokenize(&[line.to_string()], Some(lang))[0]
            .iter()
            .any(|token| token.kind == kind)
    }

    #[test]
    fn new_language_aliases_highlight_tokens() {
        for (lang, line, kind) in [
            ("haskell", "main = do print True", TokenKind::Keyword),
            ("bcpl", "LET start BE VALOF", TokenKind::Keyword),
            (
                "kotlin",
                "data class User(val name: String)",
                TokenKind::Keyword,
            ),
            ("scala", "case class User(name: String)", TokenKind::Keyword),
            (
                "csharp",
                "public record User(string Name);",
                TokenKind::Keyword,
            ),
            (
                "terraform",
                "resource \"aws_s3_bucket\" \"logs\" {}",
                TokenKind::Keyword,
            ),
            (
                "dockerfile",
                "FROM rust:latest AS build",
                TokenKind::Keyword,
            ),
            (
                "powershell",
                "function Invoke-Thing { return $true }",
                TokenKind::Keyword,
            ),
            (
                "graphql",
                "query UserQuery { user { id } }",
                TokenKind::Keyword,
            ),
            ("http", "GET /health HTTP/1.1", TokenKind::Keyword),
            ("properties", "feature.enabled=true", TokenKind::Keyword),
            ("vue", "<template><div /></template>", TokenKind::Default),
            (
                "svelte",
                "<script>let count = 1;</script>",
                TokenKind::Default,
            ),
            ("pl1", "HELLO: PROC OPTIONS(MAIN);", TokenKind::Keyword),
            ("plx", "HELLO: PROC OPTIONS(MAIN);", TokenKind::Keyword),
            ("rpg", "C           *IN99     IFEQ *OFF", TokenKind::Keyword),
            (
                "rpg2",
                "C                     EXCPTDETAIL",
                TokenKind::Keyword,
            ),
            ("rpg3", "C                     ENDIF", TokenKind::Keyword),
            ("rpgle", "ctl-opt dftactgrp(*no);", TokenKind::Keyword),
            ("rpgfree", "if %trim(customer) <> '';", TokenKind::Attribute),
            ("cl", "CHGVAR VAR(&LIB) VALUE('*LIBL')", TokenKind::Keyword),
            ("clle", "DCL VAR(&NAME) TYPE(*CHAR)", TokenKind::Attribute),
        ] {
            assert!(
                has_kind(lang, line, kind),
                "{lang} did not produce {kind:?}"
            );
        }
    }

    #[test]
    fn special_languages_are_line_aware() {
        assert!(has_kind("diff", "+added line", TokenKind::String));
        assert!(has_kind("diff", "-removed line", TokenKind::Attribute));
        assert!(has_kind("markdown", "# Heading", TokenKind::Keyword));
        assert!(has_kind("bf", "++[>++<-]", TokenKind::Keyword));
    }

    #[test]
    fn rpg_fixed_column_comments_are_comments() {
        let tokens = tokenize(
            &["      * printer spacing used to matter".to_string()],
            Some("rpg"),
        );
        assert_eq!(tokens[0].len(), 1);
        assert_eq!(tokens[0][0].kind, TokenKind::Comment);
    }
}
