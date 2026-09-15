//! Bounded syntax highlighting for fenced code blocks: a hand-written tokenizer driven by
//! small per-language tables. No regex engine, grammar files, dynamic loading or recursion;
//! every pass is a single forward scan over the (already size-limited) block text.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Token {
	Plain,
	Keyword,
	Type,
	Function,
	String,
	Comment,
	Number,
	Constant,
	/// Decorators, macros, preprocessor lines, markup attribute names, CSS properties, keys.
	Attribute,
	/// Markup element names.
	Tag,
	Punctuation,
	Added,
	Removed,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Language {
	Rust,
	JavaScript,
	TypeScript,
	Python,
	Go,
	Java,
	Kotlin,
	Swift,
	C,
	Cpp,
	CSharp,
	Php,
	Ruby,
	Lua,
	Shell,
	Json,
	Yaml,
	Toml,
	Sql,
	Html,
	Css,
	Diff,
	Dart,
	Zig,
}

/// One highlighted byte range; ranges are contiguous and cover the whole text.
pub type Segment = (u32, u32, Token);

impl Language {
	/// Resolve a fence info tag (case-insensitive, common aliases) to a known language.
	pub fn from_tag(tag: &str) -> Option<Self> {
		Some(match tag.to_ascii_lowercase().as_str() {
			"rust" | "rs" => Self::Rust,
			"js" | "javascript" | "jsx" | "mjs" | "cjs" | "node" => Self::JavaScript,
			"ts" | "typescript" | "tsx" | "mts" => Self::TypeScript,
			"py" | "python" | "python3" | "py3" => Self::Python,
			"go" | "golang" => Self::Go,
			"java" => Self::Java,
			"kt" | "kotlin" | "kts" => Self::Kotlin,
			"swift" => Self::Swift,
			"c" | "h" => Self::C,
			"cpp" | "c++" | "cc" | "cxx" | "hpp" | "hh" | "hxx" => Self::Cpp,
			"cs" | "csharp" | "c#" => Self::CSharp,
			"php" => Self::Php,
			"rb" | "ruby" => Self::Ruby,
			"lua" => Self::Lua,
			"sh" | "bash" | "zsh" | "shell" | "fish" | "console" | "shellsession" => Self::Shell,
			"json" | "jsonc" | "json5" => Self::Json,
			"yaml" | "yml" => Self::Yaml,
			"toml" | "ini" | "cfg" => Self::Toml,
			"sql" | "mysql" | "postgres" | "postgresql" | "sqlite" | "psql" => Self::Sql,
			"html" | "htm" | "xml" | "svg" | "vue" | "xhtml" | "xaml" | "jsx-html" => Self::Html,
			"css" | "scss" | "less" => Self::Css,
			"diff" | "patch" => Self::Diff,
			"dart" => Self::Dart,
			"zig" => Self::Zig,
			_ => return None,
		})
	}
	pub fn name(self) -> &'static str {
		match self {
			Self::Rust => "Rust",
			Self::JavaScript => "JavaScript",
			Self::TypeScript => "TypeScript",
			Self::Python => "Python",
			Self::Go => "Go",
			Self::Java => "Java",
			Self::Kotlin => "Kotlin",
			Self::Swift => "Swift",
			Self::C => "C",
			Self::Cpp => "C++",
			Self::CSharp => "C#",
			Self::Php => "PHP",
			Self::Ruby => "Ruby",
			Self::Lua => "Lua",
			Self::Shell => "Shell",
			Self::Json => "JSON",
			Self::Yaml => "YAML",
			Self::Toml => "TOML",
			Self::Sql => "SQL",
			Self::Html => "HTML",
			Self::Css => "CSS",
			Self::Diff => "Diff",
			Self::Dart => "Dart",
			Self::Zig => "Zig",
		}
	}
}

/// Tokenize `code`. The result covers every byte exactly once, in order.
pub fn tokenize(language: Language, code: &str) -> Vec<Segment> {
	let mut out = Segments::default();
	match language {
		Language::Html => html(code, &mut out),
		Language::Css => css(code, &mut out),
		Language::Diff => lines(code, &mut out, diff_line),
		Language::Yaml => lines(code, &mut out, yaml_line),
		Language::Toml => lines(code, &mut out, toml_line),
		Language::Json => generic(&JSON, code, &mut out),
		other => generic(syntax(other), code, &mut out),
	}
	out.finish(code.len())
}

#[derive(Default)]
struct Segments {
	segments: Vec<Segment>,
	end: usize,
}
impl Segments {
	fn push(&mut self, start: usize, end: usize, token: Token) {
		debug_assert!(start >= self.end && end >= start);
		if start > self.end {
			self.push(self.end, start, Token::Plain);
		}
		if end == start {
			return;
		}
		match self.segments.last_mut() {
			Some((_, last_end, last)) if *last == token && *last_end as usize == start => {
				*last_end = end as u32;
			}
			_ => self.segments.push((start as u32, end as u32, token)),
		}
		self.end = end;
	}
	fn finish(mut self, len: usize) -> Vec<Segment> {
		if self.end < len {
			self.push(self.end, len, Token::Plain);
		}
		self.segments
	}
}

struct Syntax {
	line_comment: &'static [&'static str],
	block_comment: Option<(&'static str, &'static str)>,
	/// Quote bytes that open a string literal.
	quotes: &'static [u8],
	/// `"""`/`'''` strings that may span lines.
	triple: bool,
	keywords: &'static [&'static str],
	types: &'static [&'static str],
	constants: &'static [&'static str],
	/// `@name` decorators/annotations.
	at_attribute: bool,
	/// `#[...]` and `#![...]` attributes, `name!` macros, `'a` lifetimes.
	rust: bool,
	/// `#include` style lines.
	preprocessor: bool,
	/// `$name` variables.
	dollar: bool,
	case_insensitive: bool,
	/// Capitalised identifiers are types.
	capital_types: bool,
	/// Double-quoted strings followed by `:` are object keys.
	json_keys: bool,
}

const NONE: &[&str] = &[];
const C_CONSTANTS: &[&str] = &["true", "false", "NULL", "nullptr"];

static RUST: Syntax = Syntax {
	line_comment: &["//"],
	block_comment: Some(("/*", "*/")),
	quotes: b"\"",
	triple: false,
	keywords: &[
		"as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum",
		"extern", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut",
		"pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait", "type",
		"unsafe", "use", "where", "while", "yield",
	],
	types: &[
		"bool", "char", "f32", "f64", "i8", "i16", "i32", "i64", "i128", "isize", "str", "u8",
		"u16", "u32", "u64", "u128", "usize", "String", "Vec", "Option", "Result", "Box", "Rc",
		"Arc",
	],
	constants: &["true", "false", "None", "Some", "Ok", "Err"],
	at_attribute: false,
	rust: true,
	preprocessor: false,
	dollar: false,
	case_insensitive: false,
	capital_types: true,
	json_keys: false,
};
static JAVASCRIPT: Syntax = Syntax {
	line_comment: &["//"],
	block_comment: Some(("/*", "*/")),
	quotes: b"\"'`",
	triple: false,
	keywords: &[
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
		"declare",
		"default",
		"delete",
		"do",
		"else",
		"enum",
		"export",
		"extends",
		"finally",
		"for",
		"from",
		"function",
		"get",
		"if",
		"implements",
		"import",
		"in",
		"instanceof",
		"interface",
		"keyof",
		"let",
		"namespace",
		"new",
		"of",
		"private",
		"protected",
		"public",
		"readonly",
		"return",
		"satisfies",
		"set",
		"static",
		"super",
		"switch",
		"this",
		"throw",
		"try",
		"type",
		"typeof",
		"var",
		"void",
		"while",
		"with",
		"yield",
	],
	types: &[
		"any", "bigint", "boolean", "never", "number", "object", "string", "symbol", "unknown",
		"Array", "Promise", "Map", "Set", "Record", "Partial", "Date", "Error", "Object", "JSON",
		"Math", "console", "document", "window",
	],
	constants: &["true", "false", "null", "undefined", "NaN", "Infinity"],
	at_attribute: true,
	rust: false,
	preprocessor: false,
	dollar: false,
	case_insensitive: false,
	capital_types: true,
	json_keys: false,
};
static PYTHON: Syntax = Syntax {
	line_comment: &["#"],
	block_comment: None,
	quotes: b"\"'",
	triple: true,
	keywords: &[
		"and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del",
		"elif", "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is",
		"lambda", "match", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while",
		"with", "yield", "case", "self", "cls",
	],
	types: &[
		"int",
		"float",
		"str",
		"bool",
		"list",
		"dict",
		"set",
		"tuple",
		"bytes",
		"object",
		"type",
		"range",
		"print",
		"len",
		"enumerate",
		"zip",
		"isinstance",
		"super",
		"Exception",
	],
	constants: &["True", "False", "None"],
	at_attribute: true,
	rust: false,
	preprocessor: false,
	dollar: false,
	case_insensitive: false,
	capital_types: true,
	json_keys: false,
};
static GO: Syntax = Syntax {
	line_comment: &["//"],
	block_comment: Some(("/*", "*/")),
	quotes: b"\"'`",
	triple: false,
	keywords: &[
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
	],
	types: &[
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
		"any",
		"make",
		"new",
		"len",
		"cap",
		"append",
		"panic",
		"println",
	],
	constants: &["true", "false", "nil", "iota"],
	at_attribute: false,
	rust: false,
	preprocessor: false,
	dollar: false,
	case_insensitive: false,
	capital_types: true,
	json_keys: false,
};
static JAVA: Syntax = Syntax {
	line_comment: &["//"],
	block_comment: Some(("/*", "*/")),
	quotes: b"\"'",
	triple: true,
	keywords: &[
		"abstract",
		"assert",
		"break",
		"case",
		"catch",
		"class",
		"continue",
		"default",
		"do",
		"else",
		"enum",
		"extends",
		"final",
		"finally",
		"for",
		"if",
		"implements",
		"import",
		"instanceof",
		"interface",
		"native",
		"new",
		"package",
		"private",
		"protected",
		"public",
		"record",
		"return",
		"sealed",
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
		"var",
		"volatile",
		"while",
		"yield",
		// Kotlin
		"fun",
		"val",
		"when",
		"object",
		"companion",
		"data",
		"override",
		"open",
		"is",
		"in",
		"as",
		"typealias",
		"suspend",
		"lateinit",
		"init",
		"internal",
		"inline",
		"reified",
		"sealed",
		"by",
		"get",
		"set",
		"constructor",
		"where",
		"out",
	],
	types: &[
		"boolean", "byte", "char", "double", "float", "int", "long", "short", "void", "String",
		"Integer", "Long", "Boolean", "Double", "List", "Map", "Set", "Object", "Unit", "Int",
		"Any", "Nothing",
	],
	constants: &["true", "false", "null"],
	at_attribute: true,
	rust: false,
	preprocessor: false,
	dollar: false,
	case_insensitive: false,
	capital_types: true,
	json_keys: false,
};
static SWIFT: Syntax = Syntax {
	line_comment: &["//"],
	block_comment: Some(("/*", "*/")),
	quotes: b"\"",
	triple: true,
	keywords: &[
		"as",
		"associatedtype",
		"async",
		"await",
		"break",
		"case",
		"catch",
		"class",
		"continue",
		"default",
		"defer",
		"deinit",
		"do",
		"else",
		"enum",
		"extension",
		"fallthrough",
		"final",
		"for",
		"func",
		"guard",
		"if",
		"import",
		"in",
		"indirect",
		"init",
		"inout",
		"internal",
		"is",
		"lazy",
		"let",
		"mutating",
		"open",
		"operator",
		"override",
		"private",
		"protocol",
		"public",
		"repeat",
		"rethrows",
		"return",
		"self",
		"Self",
		"some",
		"static",
		"struct",
		"subscript",
		"super",
		"switch",
		"throw",
		"throws",
		"try",
		"typealias",
		"var",
		"weak",
		"where",
		"while",
		"actor",
		"any",
	],
	types: &[
		"Int",
		"UInt",
		"Double",
		"Float",
		"Bool",
		"String",
		"Character",
		"Array",
		"Dictionary",
		"Set",
		"Optional",
		"Void",
		"Never",
		"Error",
		"Result",
	],
	constants: &["true", "false", "nil"],
	at_attribute: true,
	rust: false,
	preprocessor: true,
	dollar: false,
	case_insensitive: false,
	capital_types: true,
	json_keys: false,
};
static C: Syntax = Syntax {
	line_comment: &["//"],
	block_comment: Some(("/*", "*/")),
	quotes: b"\"'",
	triple: false,
	keywords: &[
		"auto",
		"break",
		"case",
		"catch",
		"class",
		"const",
		"constexpr",
		"consteval",
		"continue",
		"default",
		"delete",
		"do",
		"else",
		"enum",
		"explicit",
		"export",
		"extern",
		"final",
		"for",
		"friend",
		"goto",
		"if",
		"inline",
		"mutable",
		"namespace",
		"new",
		"noexcept",
		"operator",
		"override",
		"private",
		"protected",
		"public",
		"register",
		"return",
		"sizeof",
		"static",
		"static_assert",
		"static_cast",
		"dynamic_cast",
		"reinterpret_cast",
		"const_cast",
		"struct",
		"switch",
		"template",
		"this",
		"throw",
		"try",
		"typedef",
		"typename",
		"union",
		"using",
		"virtual",
		"volatile",
		"while",
		"co_await",
		"co_return",
		"co_yield",
		"concept",
		"requires",
	],
	types: &[
		"bool",
		"char",
		"double",
		"float",
		"int",
		"long",
		"short",
		"signed",
		"unsigned",
		"void",
		"wchar_t",
		"size_t",
		"ssize_t",
		"int8_t",
		"int16_t",
		"int32_t",
		"int64_t",
		"uint8_t",
		"uint16_t",
		"uint32_t",
		"uint64_t",
		"uintptr_t",
		"intptr_t",
		"std",
		"string",
		"vector",
		"map",
		"FILE",
	],
	constants: C_CONSTANTS,
	at_attribute: false,
	rust: false,
	preprocessor: true,
	dollar: false,
	case_insensitive: false,
	capital_types: false,
	json_keys: false,
};
static CSHARP: Syntax = Syntax {
	line_comment: &["//"],
	block_comment: Some(("/*", "*/")),
	quotes: b"\"'",
	triple: true,
	keywords: &[
		"abstract",
		"as",
		"async",
		"await",
		"base",
		"break",
		"case",
		"catch",
		"checked",
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
		"finally",
		"fixed",
		"for",
		"foreach",
		"get",
		"goto",
		"if",
		"implicit",
		"in",
		"init",
		"interface",
		"internal",
		"is",
		"lock",
		"namespace",
		"new",
		"operator",
		"out",
		"override",
		"params",
		"partial",
		"private",
		"protected",
		"public",
		"readonly",
		"record",
		"ref",
		"return",
		"sealed",
		"set",
		"sizeof",
		"stackalloc",
		"static",
		"struct",
		"switch",
		"this",
		"throw",
		"try",
		"typeof",
		"unchecked",
		"unsafe",
		"using",
		"var",
		"virtual",
		"volatile",
		"when",
		"where",
		"while",
		"yield",
		"with",
	],
	types: &[
		"bool",
		"byte",
		"char",
		"decimal",
		"double",
		"dynamic",
		"float",
		"int",
		"long",
		"nint",
		"nuint",
		"object",
		"sbyte",
		"short",
		"string",
		"uint",
		"ulong",
		"ushort",
		"void",
		"String",
		"List",
		"Dictionary",
		"Task",
		"IEnumerable",
		"Console",
	],
	constants: &["true", "false", "null"],
	at_attribute: false,
	rust: false,
	preprocessor: true,
	dollar: false,
	case_insensitive: false,
	capital_types: true,
	json_keys: false,
};
static PHP: Syntax = Syntax {
	line_comment: &["//", "#"],
	block_comment: Some(("/*", "*/")),
	quotes: b"\"'",
	triple: false,
	keywords: &[
		"abstract",
		"and",
		"array",
		"as",
		"break",
		"callable",
		"case",
		"catch",
		"class",
		"clone",
		"const",
		"continue",
		"declare",
		"default",
		"do",
		"echo",
		"else",
		"elseif",
		"empty",
		"enum",
		"extends",
		"final",
		"finally",
		"fn",
		"for",
		"foreach",
		"function",
		"global",
		"goto",
		"if",
		"implements",
		"include",
		"include_once",
		"instanceof",
		"insteadof",
		"interface",
		"isset",
		"list",
		"match",
		"namespace",
		"new",
		"or",
		"print",
		"private",
		"protected",
		"public",
		"readonly",
		"require",
		"require_once",
		"return",
		"static",
		"switch",
		"throw",
		"trait",
		"try",
		"unset",
		"use",
		"var",
		"while",
		"xor",
		"yield",
	],
	types: &[
		"int", "float", "string", "bool", "array", "object", "mixed", "void", "self", "static",
	],
	constants: &["true", "false", "null", "TRUE", "FALSE", "NULL"],
	at_attribute: false,
	rust: false,
	preprocessor: false,
	dollar: true,
	case_insensitive: false,
	capital_types: true,
	json_keys: false,
};
static RUBY: Syntax = Syntax {
	line_comment: &["#"],
	block_comment: Some(("=begin", "=end")),
	quotes: b"\"'`",
	triple: false,
	keywords: &[
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
		"for",
		"if",
		"in",
		"module",
		"next",
		"not",
		"or",
		"redo",
		"rescue",
		"retry",
		"return",
		"self",
		"super",
		"then",
		"undef",
		"unless",
		"until",
		"when",
		"while",
		"yield",
		"require",
		"require_relative",
		"include",
		"extend",
		"attr_accessor",
		"attr_reader",
		"attr_writer",
		"private",
		"public",
		"protected",
		"raise",
		"puts",
		"lambda",
		"proc",
	],
	types: &[
		"Integer", "Float", "String", "Symbol", "Array", "Hash", "Object", "Class",
	],
	constants: &["true", "false", "nil", "__FILE__", "__LINE__"],
	at_attribute: true,
	rust: false,
	preprocessor: false,
	dollar: true,
	case_insensitive: false,
	capital_types: true,
	json_keys: false,
};
static LUA: Syntax = Syntax {
	line_comment: &["--"],
	block_comment: Some(("--[[", "]]")),
	quotes: b"\"'",
	triple: false,
	keywords: &[
		"and", "break", "do", "else", "elseif", "end", "for", "function", "goto", "if", "in",
		"local", "not", "or", "repeat", "return", "then", "until", "while",
	],
	types: &[
		"print",
		"pairs",
		"ipairs",
		"type",
		"tostring",
		"tonumber",
		"require",
		"table",
		"string",
		"math",
		"io",
		"os",
		"self",
		"setmetatable",
		"getmetatable",
		"error",
		"pcall",
	],
	constants: &["true", "false", "nil"],
	at_attribute: false,
	rust: false,
	preprocessor: false,
	dollar: false,
	case_insensitive: false,
	capital_types: false,
	json_keys: false,
};
static SHELL: Syntax = Syntax {
	line_comment: &["#"],
	block_comment: None,
	quotes: b"\"'`",
	triple: false,
	keywords: &[
		"if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case", "esac",
		"in", "function", "select", "time", "return", "exit", "export", "local", "readonly",
		"declare", "typeset", "unset", "shift", "source", "alias", "set", "end", "switch", "begin",
		"break", "continue", "trap", "eval", "exec",
	],
	types: &[
		"echo", "cd", "ls", "cat", "grep", "sed", "awk", "curl", "wget", "git", "cargo", "npm",
		"npx", "pnpm", "yarn", "node", "python", "python3", "pip", "docker", "sudo", "mkdir", "rm",
		"cp", "mv", "chmod", "chown", "tar", "ssh", "make", "cmake", "apt", "brew", "test",
		"printf", "read", "find", "xargs", "touch", "kill", "ps", "which", "env", "sleep",
	],
	constants: &["true", "false"],
	at_attribute: false,
	rust: false,
	preprocessor: false,
	dollar: true,
	case_insensitive: false,
	capital_types: false,
	json_keys: false,
};
static SQL: Syntax = Syntax {
	line_comment: &["--"],
	block_comment: Some(("/*", "*/")),
	quotes: b"'\"`",
	triple: false,
	keywords: &[
		"add",
		"all",
		"alter",
		"and",
		"any",
		"as",
		"asc",
		"begin",
		"between",
		"by",
		"case",
		"check",
		"column",
		"commit",
		"constraint",
		"create",
		"cross",
		"database",
		"default",
		"delete",
		"desc",
		"distinct",
		"drop",
		"else",
		"end",
		"except",
		"exists",
		"foreign",
		"from",
		"full",
		"group",
		"having",
		"if",
		"in",
		"index",
		"inner",
		"insert",
		"intersect",
		"into",
		"is",
		"join",
		"key",
		"left",
		"like",
		"limit",
		"not",
		"null",
		"offset",
		"on",
		"or",
		"order",
		"outer",
		"primary",
		"procedure",
		"references",
		"returning",
		"right",
		"rollback",
		"select",
		"set",
		"table",
		"then",
		"transaction",
		"truncate",
		"union",
		"unique",
		"update",
		"using",
		"values",
		"view",
		"when",
		"where",
		"with",
		"over",
		"partition",
		"window",
		"replace",
		"ilike",
		"cascade",
		"explain",
		"analyze",
		"vacuum",
	],
	types: &[
		"int",
		"integer",
		"bigint",
		"smallint",
		"serial",
		"bigserial",
		"text",
		"varchar",
		"char",
		"boolean",
		"bool",
		"date",
		"timestamp",
		"timestamptz",
		"time",
		"interval",
		"numeric",
		"decimal",
		"real",
		"float",
		"double",
		"json",
		"jsonb",
		"uuid",
		"bytea",
		"blob",
		"count",
		"sum",
		"avg",
		"min",
		"max",
		"coalesce",
		"now",
		"cast",
		"length",
		"lower",
		"upper",
		"concat",
		"substring",
		"row_number",
		"rank",
	],
	constants: &["true", "false", "null"],
	at_attribute: false,
	rust: false,
	preprocessor: false,
	dollar: false,
	case_insensitive: true,
	capital_types: false,
	json_keys: false,
};
static JSON: Syntax = Syntax {
	line_comment: &["//"],
	block_comment: Some(("/*", "*/")),
	quotes: b"\"'",
	triple: false,
	keywords: NONE,
	types: NONE,
	constants: &["true", "false", "null"],
	at_attribute: false,
	rust: false,
	preprocessor: false,
	dollar: false,
	case_insensitive: false,
	capital_types: false,
	json_keys: true,
};
static DART: Syntax = Syntax {
	line_comment: &["//"],
	block_comment: Some(("/*", "*/")),
	quotes: b"\"'",
	triple: true,
	keywords: &[
		"abstract",
		"as",
		"assert",
		"async",
		"await",
		"break",
		"case",
		"catch",
		"class",
		"const",
		"continue",
		"covariant",
		"default",
		"deferred",
		"do",
		"dynamic",
		"else",
		"enum",
		"export",
		"extends",
		"extension",
		"external",
		"factory",
		"final",
		"finally",
		"for",
		"get",
		"hide",
		"if",
		"implements",
		"import",
		"in",
		"interface",
		"is",
		"late",
		"library",
		"mixin",
		"new",
		"on",
		"operator",
		"part",
		"required",
		"rethrow",
		"return",
		"sealed",
		"set",
		"show",
		"static",
		"super",
		"switch",
		"sync",
		"this",
		"throw",
		"try",
		"typedef",
		"var",
		"void",
		"when",
		"while",
		"with",
		"yield",
	],
	types: &[
		"int",
		"double",
		"num",
		"String",
		"bool",
		"List",
		"Map",
		"Set",
		"Future",
		"Stream",
		"Object",
		"Widget",
		"BuildContext",
		"Iterable",
		"Function",
		"Never",
		"Null",
	],
	constants: &["true", "false", "null"],
	at_attribute: true,
	rust: false,
	preprocessor: false,
	dollar: false,
	case_insensitive: false,
	capital_types: true,
	json_keys: false,
};
static ZIG: Syntax = Syntax {
	line_comment: &["//"],
	block_comment: None,
	quotes: b"\"'",
	triple: false,
	keywords: &[
		"addrspace",
		"align",
		"allowzero",
		"and",
		"anyframe",
		"anytype",
		"asm",
		"async",
		"await",
		"break",
		"callconv",
		"catch",
		"comptime",
		"const",
		"continue",
		"defer",
		"else",
		"enum",
		"errdefer",
		"error",
		"export",
		"extern",
		"fn",
		"for",
		"if",
		"inline",
		"linksection",
		"noalias",
		"noinline",
		"nosuspend",
		"opaque",
		"or",
		"orelse",
		"packed",
		"pub",
		"resume",
		"return",
		"struct",
		"suspend",
		"switch",
		"test",
		"threadlocal",
		"try",
		"union",
		"unreachable",
		"usingnamespace",
		"var",
		"volatile",
		"while",
	],
	types: &[
		"bool",
		"f16",
		"f32",
		"f64",
		"f128",
		"i8",
		"i16",
		"i32",
		"i64",
		"i128",
		"isize",
		"u8",
		"u16",
		"u32",
		"u64",
		"u128",
		"usize",
		"void",
		"type",
		"anyerror",
		"noreturn",
		"comptime_int",
		"comptime_float",
		"c_int",
		"c_char",
	],
	constants: &["true", "false", "null", "undefined"],
	at_attribute: true,
	rust: false,
	preprocessor: false,
	dollar: false,
	case_insensitive: false,
	capital_types: true,
	json_keys: false,
};

fn syntax(language: Language) -> &'static Syntax {
	match language {
		Language::Rust => &RUST,
		Language::JavaScript | Language::TypeScript => &JAVASCRIPT,
		Language::Python => &PYTHON,
		Language::Go => &GO,
		Language::Java | Language::Kotlin => &JAVA,
		Language::Swift => &SWIFT,
		Language::C | Language::Cpp => &C,
		Language::CSharp => &CSHARP,
		Language::Php => &PHP,
		Language::Ruby => &RUBY,
		Language::Lua => &LUA,
		Language::Shell => &SHELL,
		Language::Sql => &SQL,
		Language::Json => &JSON,
		Language::Dart => &DART,
		Language::Zig => &ZIG,
		Language::Html | Language::Css | Language::Diff | Language::Yaml | Language::Toml => &JSON,
	}
}

fn is_ident_start(byte: u8) -> bool {
	byte.is_ascii_alphabetic() || byte == b'_' || byte >= 0x80
}
fn is_ident(byte: u8) -> bool {
	byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80
}
/// End of the identifier starting at `start`.
fn ident_end(bytes: &[u8], start: usize) -> usize {
	let mut end = start;
	while end < bytes.len() && is_ident(bytes[end]) {
		end += 1;
	}
	end
}
fn line_end(bytes: &[u8], start: usize) -> usize {
	bytes[start..]
		.iter()
		.position(|byte| *byte == b'\n')
		.map_or(bytes.len(), |offset| start + offset)
}
/// End of a quoted literal starting at `start` (the opening quote), honouring backslash
/// escapes. Single-line strings stop at a newline so an unterminated quote cannot swallow
/// the rest of the block.
fn string_end(bytes: &[u8], start: usize, quote: u8, multiline: bool) -> usize {
	let mut i = start + 1;
	while i < bytes.len() {
		match bytes[i] {
			b'\\' => i += 2,
			byte if byte == quote => return i + 1,
			b'\n' if !multiline => return i,
			_ => i += 1,
		}
	}
	bytes.len()
}
fn number_end(bytes: &[u8], start: usize) -> usize {
	let mut end = start;
	while end < bytes.len()
		&& (bytes[end].is_ascii_alphanumeric()
			|| bytes[end] == b'_'
			|| (bytes[end] == b'.' && end + 1 < bytes.len() && bytes[end + 1].is_ascii_digit()))
	{
		end += 1;
	}
	end
}
fn find(bytes: &[u8], from: usize, needle: &str) -> Option<usize> {
	let needle = needle.as_bytes();
	if needle.is_empty() || from >= bytes.len() {
		return None;
	}
	bytes[from..]
		.windows(needle.len())
		.position(|window| window == needle)
		.map(|offset| from + offset)
}
fn starts_with(bytes: &[u8], at: usize, prefix: &str) -> bool {
	bytes[at..].starts_with(prefix.as_bytes())
}
fn word_in(list: &[&str], word: &str, case_insensitive: bool) -> bool {
	if case_insensitive {
		list.iter().any(|entry| entry.eq_ignore_ascii_case(word))
	} else {
		list.contains(&word)
	}
}
fn next_non_space(bytes: &[u8], from: usize) -> Option<u8> {
	bytes[from..]
		.iter()
		.copied()
		.find(|byte| !matches!(byte, b' ' | b'\t'))
}
fn is_punctuation(byte: u8) -> bool {
	matches!(
		byte,
		b'{' | b'}'
			| b'[' | b']'
			| b'(' | b')'
			| b'<' | b'>'
			| b';' | b','
			| b'.' | b':'
			| b'=' | b'+'
			| b'-' | b'*'
			| b'/' | b'%'
			| b'&' | b'|'
			| b'^' | b'!'
			| b'~' | b'?'
	)
}
fn char_len(bytes: &[u8], at: usize) -> usize {
	match bytes[at] {
		byte if byte < 0x80 => 1,
		byte if byte >= 0xf0 => 4,
		byte if byte >= 0xe0 => 3,
		_ => 2,
	}
}

fn generic(syntax: &Syntax, code: &str, out: &mut Segments) {
	let bytes = code.as_bytes();
	let mut i = 0;
	let mut line_start = true;
	// The identifier after `fn`/`def`/`function` names a definition even before `<T>(`.
	let mut after_definer = false;
	while i < bytes.len() {
		let byte = bytes[i];
		if byte == b'\n' {
			i += 1;
			line_start = true;
			continue;
		}
		if matches!(byte, b' ' | b'\t' | b'\r') {
			i += 1;
			continue;
		}
		let at_line_start = line_start;
		line_start = false;
		let definer = std::mem::take(&mut after_definer);
		if let Some(prefix) = syntax
			.line_comment
			.iter()
			.find(|prefix| starts_with(bytes, i, prefix))
		{
			// `#` is only a comment when it cannot be a Rust attribute or C preprocessor line.
			if !(*prefix == "#" && (syntax.rust || syntax.preprocessor)) {
				let end = line_end(bytes, i);
				out.push(i, end, Token::Comment);
				i = end;
				continue;
			}
		}
		if let Some((open, close)) = syntax.block_comment
			&& starts_with(bytes, i, open)
		{
			let end = find(bytes, i + open.len(), close).map_or(bytes.len(), |at| at + close.len());
			out.push(i, end, Token::Comment);
			i = end;
			continue;
		}
		if syntax.preprocessor && byte == b'#' && at_line_start {
			let end = line_end(bytes, i);
			out.push(i, end, Token::Attribute);
			i = end;
			continue;
		}
		if syntax.rust
			&& byte == b'#'
			&& (starts_with(bytes, i, "#[") || starts_with(bytes, i, "#!["))
		{
			let limit = line_end(bytes, i);
			let end = bytes[i..limit]
				.iter()
				.position(|byte| *byte == b']')
				.map_or(limit, |offset| i + offset + 1);
			out.push(i, end, Token::Attribute);
			i = end;
			continue;
		}
		if syntax.rust && byte == b'\'' {
			// `'a` is a lifetime unless a closing quote makes it a `'a'` char literal.
			let literal_char = i + 2 < bytes.len() && bytes[i + 2] == b'\'';
			let escaped = i + 1 < bytes.len() && bytes[i + 1] == b'\\';
			if !literal_char && !escaped && i + 1 < bytes.len() && is_ident_start(bytes[i + 1]) {
				let end = ident_end(bytes, i + 1);
				out.push(i, end, Token::Type);
				i = end;
				continue;
			}
			let end = string_end(bytes, i, b'\'', false);
			out.push(i, end, Token::String);
			i = end;
			continue;
		}
		if syntax.quotes.contains(&byte) {
			let triple = syntax.triple
				&& byte != b'`'
				&& i + 2 < bytes.len()
				&& bytes[i + 1] == byte
				&& bytes[i + 2] == byte;
			let end = if triple {
				let close = [byte; 3];
				let close = std::str::from_utf8(&close).unwrap_or("\"\"\"");
				find(bytes, i + 3, close).map_or(bytes.len(), |at| at + 3)
			} else {
				string_end(bytes, i, byte, byte == b'`')
			};
			let token = if syntax.json_keys && next_non_space(bytes, end) == Some(b':') {
				Token::Attribute
			} else {
				Token::String
			};
			out.push(i, end, token);
			i = end;
			continue;
		}
		if syntax.dollar && byte == b'$' && i + 1 < bytes.len() {
			let end = match bytes[i + 1] {
				b'{' => bytes[i..line_end(bytes, i)]
					.iter()
					.position(|byte| *byte == b'}')
					.map_or(i + 2, |offset| i + offset + 1),
				next if is_ident_start(next) => ident_end(bytes, i + 1),
				b'?' | b'@' | b'#' | b'!' | b'0'..=b'9' => i + 2,
				_ => i + 1,
			};
			out.push(i, end, Token::Attribute);
			i = end;
			continue;
		}
		if syntax.at_attribute
			&& byte == b'@'
			&& i + 1 < bytes.len()
			&& is_ident_start(bytes[i + 1])
		{
			let end = ident_end(bytes, i + 1);
			out.push(i, end, Token::Attribute);
			i = end;
			continue;
		}
		if byte.is_ascii_digit()
			|| (byte == b'.' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit())
		{
			let end = number_end(bytes, i + 1);
			out.push(i, end, Token::Number);
			i = end;
			continue;
		}
		if is_ident_start(byte) {
			let end = ident_end(bytes, i);
			let word = &code[i..end];
			let token = if syntax.rust && end < bytes.len() && bytes[end] == b'!' {
				out.push(i, end + 1, Token::Attribute);
				i = end + 1;
				continue;
			} else if word_in(syntax.keywords, word, syntax.case_insensitive) {
				after_definer = matches!(word, "fn" | "func" | "def" | "function" | "fun" | "sub");
				Token::Keyword
			} else if definer {
				Token::Function
			} else if word_in(syntax.constants, word, syntax.case_insensitive) {
				Token::Constant
			} else if word_in(syntax.types, word, syntax.case_insensitive)
				|| (syntax.capital_types
					&& byte.is_ascii_uppercase()
					&& word.bytes().any(|byte| byte.is_ascii_lowercase()))
			{
				Token::Type
			} else if syntax.capital_types
				&& byte.is_ascii_uppercase()
				&& word.len() > 1
				&& word
					.bytes()
					.all(|byte| byte.is_ascii_uppercase() || byte == b'_' || byte.is_ascii_digit())
			{
				Token::Constant
			} else if next_non_space(bytes, end) == Some(b'(') {
				Token::Function
			} else {
				Token::Plain
			};
			out.push(i, end, token);
			i = end;
			continue;
		}
		if is_punctuation(byte) {
			out.push(i, i + 1, Token::Punctuation);
			i += 1;
			continue;
		}
		i += char_len(bytes, i);
	}
}

fn lines(code: &str, out: &mut Segments, mut line: impl FnMut(&str, usize, &mut Segments)) {
	let mut start = 0;
	for text in code.split_inclusive('\n') {
		let content = text.trim_end_matches(['\n', '\r']);
		line(content, start, out);
		start += text.len();
	}
}

fn diff_line(line: &str, start: usize, out: &mut Segments) {
	let end = start + line.len();
	let token = if line.starts_with("+++") || line.starts_with("---") {
		Token::Comment
	} else if line.starts_with('+') {
		Token::Added
	} else if line.starts_with('-') {
		Token::Removed
	} else if line.starts_with("@@") {
		Token::Keyword
	} else if line.starts_with("diff ") || line.starts_with("index ") {
		Token::Comment
	} else {
		Token::Plain
	};
	out.push(start, end, token);
}

/// Shared by YAML and TOML: strings, numbers, booleans and trailing `#` comments.
fn config_value(value: &str, start: usize, out: &mut Segments) {
	let bytes = value.as_bytes();
	let mut i = 0;
	while i < bytes.len() {
		let byte = bytes[i];
		if matches!(byte, b' ' | b'\t' | b',' | b'[' | b']' | b'{' | b'}') {
			if byte != b' ' && byte != b'\t' {
				out.push(start + i, start + i + 1, Token::Punctuation);
			}
			i += 1;
			continue;
		}
		if byte == b'#' && (i == 0 || matches!(bytes[i - 1], b' ' | b'\t')) {
			out.push(start + i, start + bytes.len(), Token::Comment);
			return;
		}
		if byte == b'"' || byte == b'\'' {
			let end = string_end(bytes, i, byte, false);
			out.push(start + i, start + end, Token::String);
			i = end;
			continue;
		}
		let end = bytes[i..]
			.iter()
			.position(|byte| matches!(byte, b' ' | b'\t' | b',' | b']' | b'}'))
			.map_or(bytes.len(), |offset| i + offset);
		let word = &value[i..end];
		let token =
			if matches!(
				word,
				"true"
					| "false" | "null"
					| "yes" | "no" | "on"
					| "off" | "~" | "True"
					| "False" | "None"
					| "inf" | "nan"
			) {
				Token::Constant
			} else if word.starts_with(['&', '*', '!']) {
				Token::Type
			} else if word
				.bytes()
				.next()
				.is_some_and(|b| b.is_ascii_digit() || b == b'-' || b == b'+')
				&& word.bytes().skip(1).all(|b| {
					b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'-' | b'+')
				}) && word.bytes().any(|b| b.is_ascii_digit())
			{
				Token::Number
			} else {
				Token::Plain
			};
		out.push(start + i, start + end, token);
		i = end.max(i + 1);
	}
}

fn yaml_line(line: &str, start: usize, out: &mut Segments) {
	let trimmed = line.trim_start();
	let mut offset = line.len() - trimmed.len();
	if trimmed.starts_with('#') {
		out.push(start + offset, start + line.len(), Token::Comment);
		return;
	}
	if trimmed.starts_with("---") || trimmed.starts_with("...") {
		out.push(start + offset, start + line.len(), Token::Punctuation);
		return;
	}
	let mut rest = trimmed;
	if let Some(item) = rest.strip_prefix("- ") {
		out.push(start + offset, start + offset + 1, Token::Punctuation);
		offset += 2;
		rest = item;
	} else if rest == "-" {
		out.push(start + offset, start + offset + 1, Token::Punctuation);
		return;
	}
	// `key:` or `"quoted key":` followed by space or end of line.
	let key_end = rest.bytes().enumerate().find_map(|(index, byte)| {
		(byte == b':'
			&& rest
				.as_bytes()
				.get(index + 1)
				.is_none_or(|next| *next == b' '))
		.then_some(index)
	});
	if let Some(key_end) = key_end
		&& key_end > 0
		&& !rest[..key_end].starts_with(['"', '\'', '[', '{'])
	{
		out.push(start + offset, start + offset + key_end, Token::Attribute);
		out.push(
			start + offset + key_end,
			start + offset + key_end + 1,
			Token::Punctuation,
		);
		offset += key_end + 1;
		rest = &rest[key_end + 1..];
	}
	config_value(rest, start + offset, out);
}

fn toml_line(line: &str, start: usize, out: &mut Segments) {
	let trimmed = line.trim_start();
	let mut offset = line.len() - trimmed.len();
	if trimmed.starts_with('#') {
		out.push(start + offset, start + line.len(), Token::Comment);
		return;
	}
	if trimmed.starts_with('[') {
		let end = trimmed.find(']').map_or(trimmed.len(), |i| i + 1);
		out.push(start + offset, start + offset + end, Token::Type);
		config_value(&trimmed[end..], start + offset + end, out);
		return;
	}
	let mut rest = trimmed;
	if let Some(eq) = rest.find('=')
		&& !rest[..eq].starts_with(['"', '\''])
	{
		let key = rest[..eq].trim_end();
		out.push(start + offset, start + offset + key.len(), Token::Attribute);
		out.push(
			start + offset + eq,
			start + offset + eq + 1,
			Token::Punctuation,
		);
		offset += eq + 1;
		rest = &rest[eq + 1..];
	}
	config_value(rest, start + offset, out);
}

fn html(code: &str, out: &mut Segments) {
	let bytes = code.as_bytes();
	let mut i = 0;
	while i < bytes.len() {
		if starts_with(bytes, i, "<!--") {
			let end = find(bytes, i + 4, "-->").map_or(bytes.len(), |at| at + 3);
			out.push(i, end, Token::Comment);
			i = end;
			continue;
		}
		if bytes[i] == b'&' {
			let end = ident_end(bytes, i + 1);
			if end < bytes.len() && bytes[end] == b';' && end > i + 1 {
				out.push(i, end + 1, Token::Constant);
				i = end + 1;
				continue;
			}
		}
		if bytes[i] == b'<'
			&& i + 1 < bytes.len()
			&& (is_ident_start(bytes[i + 1]) || matches!(bytes[i + 1], b'/' | b'!' | b'?'))
		{
			// `<`, `</`, `<!`, `<?` then the element name.
			let mut j = i + 1;
			if matches!(bytes[j], b'/' | b'!' | b'?') {
				j += 1;
			}
			out.push(i, j, Token::Punctuation);
			let name_end = {
				let mut end = j;
				while end < bytes.len()
					&& (is_ident(bytes[end]) || matches!(bytes[end], b'-' | b':' | b'.'))
				{
					end += 1;
				}
				end
			};
			out.push(j, name_end, Token::Tag);
			i = name_end;
			// Attributes until the closing `>`.
			while i < bytes.len() {
				let byte = bytes[i];
				if byte == b'>' {
					out.push(i, i + 1, Token::Punctuation);
					i += 1;
					break;
				}
				if starts_with(bytes, i, "/>") || starts_with(bytes, i, "?>") {
					out.push(i, i + 2, Token::Punctuation);
					i += 2;
					break;
				}
				if byte == b'"' || byte == b'\'' {
					let end = string_end(bytes, i, byte, true);
					out.push(i, end, Token::String);
					i = end;
					continue;
				}
				if byte == b'=' {
					out.push(i, i + 1, Token::Punctuation);
					i += 1;
					continue;
				}
				if is_ident_start(byte) || byte == b'@' || byte == b':' || byte == b'#' {
					let mut end = i + 1;
					while end < bytes.len()
						&& (is_ident(bytes[end]) || matches!(bytes[end], b'-' | b':' | b'.' | b'@'))
					{
						end += 1;
					}
					out.push(i, end, Token::Attribute);
					i = end;
					continue;
				}
				i += char_len(bytes, i);
			}
			continue;
		}
		i += char_len(bytes, i);
	}
}

fn css(code: &str, out: &mut Segments) {
	let bytes = code.as_bytes();
	let mut i = 0;
	let mut depth = 0_u32;
	// Inside a declaration value (after `:` within braces) until `;` or `}`.
	let mut in_value = false;
	while i < bytes.len() {
		let byte = bytes[i];
		if starts_with(bytes, i, "/*") {
			let end = find(bytes, i + 2, "*/").map_or(bytes.len(), |at| at + 2);
			out.push(i, end, Token::Comment);
			i = end;
			continue;
		}
		if starts_with(bytes, i, "//") {
			let end = line_end(bytes, i);
			out.push(i, end, Token::Comment);
			i = end;
			continue;
		}
		if byte == b'"' || byte == b'\'' {
			let end = string_end(bytes, i, byte, false);
			out.push(i, end, Token::String);
			i = end;
			continue;
		}
		match byte {
			b'{' => {
				depth += 1;
				in_value = false;
				out.push(i, i + 1, Token::Punctuation);
				i += 1;
				continue;
			}
			b'}' => {
				depth = depth.saturating_sub(1);
				in_value = false;
				out.push(i, i + 1, Token::Punctuation);
				i += 1;
				continue;
			}
			b';' | b',' | b'>' | b'+' | b'~' | b'(' | b')' | b'=' => {
				if byte == b';' {
					in_value = false;
				}
				out.push(i, i + 1, Token::Punctuation);
				i += 1;
				continue;
			}
			b':' if depth > 0 && !in_value => {
				in_value = true;
				out.push(i, i + 1, Token::Punctuation);
				i += 1;
				continue;
			}
			b' ' | b'\t' | b'\n' | b'\r' => {
				i += 1;
				continue;
			}
			_ => {}
		}
		if byte == b'@' {
			let end = ident_end(bytes, i + 1);
			let end = bytes[end..]
				.iter()
				.position(|byte| matches!(byte, b' ' | b'{' | b';' | b'\n' | b'('))
				.map_or(bytes.len(), |offset| end + offset)
				.max(end);
			out.push(i, end, Token::Keyword);
			i = end;
			continue;
		}
		if byte == b'!' {
			let end = ident_end(bytes, i + 1);
			out.push(i, end, Token::Keyword);
			i = end;
			continue;
		}
		if byte == b'#' {
			let end = ident_end(bytes, i + 1);
			out.push(i, end, if in_value { Token::Number } else { Token::Type });
			i = end;
			continue;
		}
		if depth == 0 || !in_value {
			// Selector or property position.
			if byte == b'.' {
				let mut end = ident_end(bytes, i + 1);
				while end < bytes.len() && bytes[end] == b'-' {
					end = ident_end(bytes, end + 1);
				}
				out.push(i, end, Token::Type);
				i = end;
				continue;
			}
			if byte == b':' {
				let mut j = i + 1;
				while j < bytes.len() && bytes[j] == b':' {
					j += 1;
				}
				let mut end = ident_end(bytes, j);
				while end < bytes.len() && bytes[end] == b'-' {
					end = ident_end(bytes, end + 1);
				}
				out.push(i, end, Token::Keyword);
				i = end;
				continue;
			}
			if byte == b'[' {
				let end = bytes[i..]
					.iter()
					.position(|byte| *byte == b']')
					.map_or(bytes.len(), |offset| i + offset + 1);
				out.push(i, end, Token::Attribute);
				i = end;
				continue;
			}
			if is_ident_start(byte) || byte == b'-' || byte == b'*' {
				let mut end = i + 1;
				while end < bytes.len() && (is_ident(bytes[end]) || bytes[end] == b'-') {
					end += 1;
				}
				out.push(
					i,
					end,
					if depth > 0 {
						Token::Attribute
					} else {
						Token::Tag
					},
				);
				i = end;
				continue;
			}
		} else {
			// Declaration value.
			if byte.is_ascii_digit()
				|| ((byte == b'.' || byte == b'-')
					&& i + 1 < bytes.len()
					&& bytes[i + 1].is_ascii_digit())
			{
				let mut end = number_end(bytes, i + 1);
				if end < bytes.len() && bytes[end] == b'%' {
					end += 1;
				}
				out.push(i, end, Token::Number);
				i = end;
				continue;
			}
			if is_ident_start(byte) || byte == b'-' {
				let mut end = i + 1;
				while end < bytes.len() && (is_ident(bytes[end]) || bytes[end] == b'-') {
					end += 1;
				}
				let token = if end < bytes.len() && bytes[end] == b'(' {
					Token::Function
				} else if bytes[i] == b'-' && starts_with(bytes, i, "--") {
					Token::Attribute
				} else {
					Token::Plain
				};
				out.push(i, end, token);
				i = end;
				continue;
			}
		}
		i += char_len(bytes, i);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn tokens(language: Language, code: &str) -> Vec<(&str, Token)> {
		let segments = tokenize(language, code);
		let mut covered = 0;
		for (start, end, _) in &segments {
			assert_eq!(*start as usize, covered, "segments must be contiguous");
			covered = *end as usize;
		}
		assert_eq!(covered, code.len(), "segments must cover the whole text");
		segments
			.into_iter()
			.map(|(start, end, token)| (&code[start as usize..end as usize], token))
			.filter(|(_, token)| *token != Token::Plain)
			.collect()
	}

	#[test]
	fn resolves_common_aliases() {
		assert_eq!(Language::from_tag("JS"), Some(Language::JavaScript));
		assert_eq!(Language::from_tag("rs"), Some(Language::Rust));
		assert_eq!(Language::from_tag("c++"), Some(Language::Cpp));
		assert_eq!(Language::from_tag("elixir"), None);
	}

	#[test]
	fn rust_tokens_cover_keywords_macros_lifetimes_and_strings() {
		let toks = tokens(
			Language::Rust,
			"#[derive(Debug)]\nfn main<'a>() -> Option<&'a str> { let x: u32 = 0x1F; println!(\"hi {x}\"); // note\n 'c' }",
		);
		assert!(toks.contains(&("#[derive(Debug)]", Token::Attribute)));
		assert!(toks.contains(&("fn", Token::Keyword)));
		assert!(toks.contains(&("main", Token::Function)));
		assert!(toks.contains(&("'a", Token::Type)));
		assert!(toks.contains(&("Option", Token::Type)));
		assert!(toks.contains(&("u32", Token::Type)));
		assert!(toks.contains(&("0x1F", Token::Number)));
		assert!(toks.contains(&("println!", Token::Attribute)));
		assert!(toks.contains(&("\"hi {x}\"", Token::String)));
		assert!(toks.contains(&("// note", Token::Comment)));
		assert!(toks.contains(&("'c'", Token::String)));
	}

	#[test]
	fn javascript_template_strings_span_lines_and_unterminated_quotes_stop_at_newline() {
		let toks = tokens(
			Language::JavaScript,
			"const s = `a\nb`;\nlet t = \"open\nnext();",
		);
		assert!(toks.contains(&("`a\nb`", Token::String)));
		assert!(toks.contains(&("\"open", Token::String)));
		assert!(toks.contains(&("next", Token::Function)));
		assert!(toks.contains(&("const", Token::Keyword)));
	}

	#[test]
	fn python_decorators_triple_strings_and_comments() {
		let toks = tokens(
			Language::Python,
			"@dataclass\nclass A:\n    \"\"\"doc\nmore\"\"\"\n    x = None  # c",
		);
		assert!(toks.contains(&("@dataclass", Token::Attribute)));
		assert!(toks.contains(&("class", Token::Keyword)));
		assert!(toks.contains(&("\"\"\"doc\nmore\"\"\"", Token::String)));
		assert!(toks.contains(&("None", Token::Constant)));
		assert!(toks.contains(&("# c", Token::Comment)));
	}

	#[test]
	fn json_keys_differ_from_values() {
		let toks = tokens(Language::Json, "{\"a\": \"b\", \"n\": 1.5, \"t\": true}");
		assert!(toks.contains(&("\"a\"", Token::Attribute)));
		assert!(toks.contains(&("\"b\"", Token::String)));
		assert!(toks.contains(&("1.5", Token::Number)));
		assert!(toks.contains(&("true", Token::Constant)));
	}

	#[test]
	fn sql_is_case_insensitive_and_c_has_preprocessor_lines() {
		let toks = tokens(Language::Sql, "SELECT id FROM t WHERE x = 'a';");
		assert!(toks.contains(&("SELECT", Token::Keyword)));
		assert!(toks.contains(&("WHERE", Token::Keyword)));
		assert!(toks.contains(&("'a'", Token::String)));
		let toks = tokens(
			Language::C,
			"#include <stdio.h>\nint main(void) { return 0; }",
		);
		assert!(toks.contains(&("#include <stdio.h>", Token::Attribute)));
		assert!(toks.contains(&("int", Token::Type)));
		assert!(toks.contains(&("main", Token::Function)));
	}

	#[test]
	fn markup_and_config_languages() {
		let toks = tokens(
			Language::Html,
			"<!-- c --><div class=\"x\" data-a='1'>&amp;</div>",
		);
		assert!(toks.contains(&("<!-- c -->", Token::Comment)));
		assert!(toks.contains(&("div", Token::Tag)));
		assert!(toks.contains(&("class", Token::Attribute)));
		assert!(toks.contains(&("\"x\"", Token::String)));
		assert!(toks.contains(&("&amp;", Token::Constant)));
		let toks = tokens(
			Language::Css,
			".a:hover { color: #fff; width: 10px; } @media (x) {}",
		);
		assert!(toks.contains(&(".a", Token::Type)));
		assert!(toks.contains(&(":hover", Token::Keyword)));
		assert!(toks.contains(&("color", Token::Attribute)));
		assert!(toks.contains(&("#fff", Token::Number)));
		assert!(toks.contains(&("10px", Token::Number)));
		assert!(toks.contains(&("@media", Token::Keyword)));
		let toks = tokens(
			Language::Yaml,
			"# top\nkey: value\nlist:\n  - 3\n  - \"s\" # end\nflag: true",
		);
		assert!(toks.contains(&("# top", Token::Comment)));
		assert!(toks.contains(&("key", Token::Attribute)));
		assert!(toks.contains(&("3", Token::Number)));
		assert!(toks.contains(&("\"s\"", Token::String)));
		assert!(toks.contains(&("# end", Token::Comment)));
		assert!(toks.contains(&("true", Token::Constant)));
		let toks = tokens(Language::Toml, "[package]\nname = \"x\" # c\nn = 12");
		assert!(toks.contains(&("[package]", Token::Type)));
		assert!(toks.contains(&("name", Token::Attribute)));
		assert!(toks.contains(&("\"x\"", Token::String)));
		assert!(toks.contains(&("12", Token::Number)));
		let toks = tokens(
			Language::Diff,
			"--- a\n+++ b\n@@ -1 +1 @@\n-old\n+new\n same",
		);
		assert!(toks.contains(&("--- a", Token::Comment)));
		assert!(toks.contains(&("@@ -1 +1 @@", Token::Keyword)));
		assert!(toks.contains(&("-old", Token::Removed)));
		assert!(toks.contains(&("+new", Token::Added)));
	}

	#[test]
	fn arbitrary_bytes_never_panic_and_stay_covered() {
		let junk =
			"\"unterminated `x\n\\\u{1F600}#[ 'a ... ${ $1 @ 0. .5 <a href=\"\n /* open\n<!-- open";
		for language in [
			Language::Rust,
			Language::JavaScript,
			Language::Python,
			Language::Shell,
			Language::Json,
			Language::Yaml,
			Language::Toml,
			Language::Html,
			Language::Css,
			Language::Diff,
			Language::Sql,
			Language::Php,
		] {
			tokens(language, junk);
			tokens(language, "");
			tokens(language, "\u{1F600}");
		}
	}
}
