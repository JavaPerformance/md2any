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
        "ruby" | "rb" => Some(&LANG_RUBY),
        "bash" | "sh" | "shell" | "zsh" | "fish" => Some(&LANG_BASH),
        "sql" | "postgres" | "postgresql" | "mysql" | "sqlite" => Some(&LANG_SQL),
        "json" => Some(&LANG_JSON),
        "yaml" | "yml" => Some(&LANG_YAML),
        "toml" => Some(&LANG_TOML),
        "html" | "xml" | "svg" => Some(&LANG_HTML),
        "css" | "scss" | "less" => Some(&LANG_CSS),
        "cobol" | "cob" | "cbl" => Some(&LANG_COBOL),
        "jcl" => Some(&LANG_JCL),
        "rexx" | "rex" => Some(&LANG_REXX),
        "pli" | "pl1" | "pl/i" | "pli390" => Some(&LANG_PLI),
        "hlasm" | "asm" | "asm370" | "asm390" | "s390asm" | "ibmasm" => Some(&LANG_HLASM),
        "db2" | "db2sql" | "db2ddl" => Some(&LANG_DB2),
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

#[derive(Default, Clone, Copy)]
pub struct State {
    in_block_comment: bool,
    in_triple_string: bool,
}

pub fn tokenize(lines: &[String], lang_name: Option<&str>) -> Vec<Vec<Token>> {
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
