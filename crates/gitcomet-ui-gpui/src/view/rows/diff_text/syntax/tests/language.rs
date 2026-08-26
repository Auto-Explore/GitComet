use super::*;

/// Every wired language, a snippet of it, and the kinds it must colour.
///
/// The bar is what a reader looks for rather than what a grammar happens to
/// offer: comments must not read as code, literals must not read as names,
/// and brackets must not be flat. Data formats are held to what they have --
/// no functions in JSON -- but a programming language that cannot colour a
/// call or a bracket has a gap worth fixing, not a style to accept.
///
/// Adding a grammar without adding a row here leaves it unguarded, which is
/// how Objective-C shipped with no comment colour at all.
const LANGUAGE_BASELINES: &[(DiffSyntaxLanguage, &str, &[SyntaxTokenKind])] = {
    use SyntaxTokenKind as K;
    &[
        (
            DiffSyntaxLanguage::Rust,
            "// c\nfn main() {\n    let x = \"s\";\n    let n = 1;\n    f(x);\n}\n",
            &[
                K::Comment,
                K::String,
                K::Number,
                K::Keyword,
                K::Function,
                K::PunctuationBracket,
            ],
        ),
        (
            DiffSyntaxLanguage::Python,
            "# c\ndef main():\n    x = \"s\"\n    n = 1\n    f(x)\n",
            &[
                K::Comment,
                K::String,
                K::Number,
                K::Keyword,
                K::Function,
                K::PunctuationBracket,
            ],
        ),
        (
            DiffSyntaxLanguage::JavaScript,
            "// c\nfunction main() {\n  const x = \"s\";\n  const n = 1;\n  f(x);\n}\n",
            &[
                K::Comment,
                K::String,
                K::Number,
                K::Keyword,
                K::Function,
                K::PunctuationBracket,
            ],
        ),
        (
            DiffSyntaxLanguage::TypeScript,
            "// c\nfunction main(): void {\n  const x: string = \"s\";\n  const n = 1;\n  f(x);\n}\n",
            &[
                K::Comment,
                K::String,
                K::Number,
                K::Keyword,
                K::Function,
                K::PunctuationBracket,
                K::Type,
            ],
        ),
        (
            DiffSyntaxLanguage::Tsx,
            "// c\nconst a = <div id=\"x\">{f(1)}</div>;\n",
            &[
                K::Comment,
                K::String,
                K::Number,
                K::Tag,
                K::PunctuationBracket,
            ],
        ),
        (
            DiffSyntaxLanguage::Go,
            "// c\nfunc main() {\n\tx := \"s\"\n\tn := 1\n\tf(x)\n}\n",
            &[
                K::Comment,
                K::String,
                K::Number,
                K::Keyword,
                K::Function,
                K::PunctuationBracket,
            ],
        ),
        (
            DiffSyntaxLanguage::C,
            "// c\nint main(void) {\n  const char *x = \"s\";\n  int n = 1;\n  f(x);\n}\n",
            &[
                K::Comment,
                K::String,
                K::Number,
                K::Keyword,
                K::Function,
                K::PunctuationBracket,
                K::Type,
            ],
        ),
        (
            DiffSyntaxLanguage::Cpp,
            "// c\nclass A {};\nint main() {\n  std::string x = \"s\";\n  return 1;\n}\n",
            &[
                K::Comment,
                K::String,
                K::Number,
                K::Keyword,
                K::Function,
                K::PunctuationBracket,
                K::Type,
            ],
        ),
        (
            DiffSyntaxLanguage::ObjectiveC,
            "// c\n@implementation Foo\n- (void)bar {\n  NSString *s = @\"hi\";\n  int n = 42;\n}\n@end\n",
            &[
                K::Comment,
                K::String,
                K::Number,
                K::Keyword,
                K::PunctuationBracket,
                K::Type,
            ],
        ),
        (
            DiffSyntaxLanguage::CSharp,
            "// c\nclass Foo {\n  void Bar() {\n    var x = \"s\";\n    int n = 1;\n  }\n}\n",
            &[
                K::Comment,
                K::String,
                K::Number,
                K::Keyword,
                K::PunctuationBracket,
                K::Type,
            ],
        ),
        (
            DiffSyntaxLanguage::Java,
            "// c\nclass Foo {\n  int count = 1;\n  void bar() {\n    f(\"s\");\n  }\n}\n",
            &[
                K::Comment,
                K::String,
                K::Number,
                K::Keyword,
                K::PunctuationBracket,
                K::Type,
            ],
        ),
        (
            DiffSyntaxLanguage::Kotlin,
            "// c\nfun main() {\n  val x = \"s\"\n  val n = 1\n  f(x)\n}\n",
            &[
                K::Comment,
                K::String,
                K::Number,
                K::Keyword,
                K::Function,
                K::PunctuationBracket,
            ],
        ),
        (
            DiffSyntaxLanguage::Scala,
            "// c\nobject Foo {\n  def bar(): Unit = {\n    val x = \"s\"\n    val n = 1\n  }\n}\n",
            &[
                K::Comment,
                K::String,
                K::Number,
                K::Keyword,
                K::PunctuationBracket,
            ],
        ),
        (
            DiffSyntaxLanguage::Groovy,
            "// c\nclass Foo {\n  int count = 1\n  def bar() { f(\"s\") }\n}\n",
            &[
                K::Comment,
                K::String,
                K::Number,
                K::Keyword,
                K::PunctuationBracket,
            ],
        ),
        (
            DiffSyntaxLanguage::Php,
            "<?php\n// c\nclass Foo {\n  public $count = 1;\n  function bar() { return f(\"s\"); }\n}\n",
            &[
                K::Comment,
                K::String,
                K::Number,
                K::Keyword,
                K::PunctuationBracket,
            ],
        ),
        (
            DiffSyntaxLanguage::Ruby,
            "# c\nclass Foo\n  def bar\n    x = \"s\"\n    n = 1\n    f(x)\n  end\nend\n",
            &[
                K::Comment,
                K::String,
                K::Number,
                K::Keyword,
                K::PunctuationBracket,
            ],
        ),
        (
            DiffSyntaxLanguage::Swift,
            "// c\nfunc main() {\n  let x = \"s\"\n  let n = 1\n  f(x)\n}\n",
            &[
                K::Comment,
                K::String,
                K::Number,
                K::Keyword,
                K::Function,
                K::PunctuationBracket,
            ],
        ),
        (
            DiffSyntaxLanguage::Dart,
            "// c\nvoid main() {\n  var x = \"s\";\n  var n = 1;\n  f(x);\n}\n",
            &[
                K::Comment,
                K::String,
                K::Number,
                K::Keyword,
                K::Function,
                K::PunctuationBracket,
            ],
        ),
        (
            DiffSyntaxLanguage::Lua,
            "-- c\nlocal function main()\n  local x = \"s\"\n  local n = 1\n  f(x)\nend\n",
            &[
                K::Comment,
                K::String,
                K::Number,
                K::Keyword,
                K::Function,
                K::PunctuationBracket,
            ],
        ),
        (
            DiffSyntaxLanguage::Bash,
            "# c\nmain() {\n  x=\"s\"\n  n=1\n  f \"$x\"\n}\n",
            &[K::Comment, K::String, K::Number, K::PunctuationBracket],
        ),
        (
            DiffSyntaxLanguage::PowerShell,
            "# c\nfunction Get-Thing {\n  $x = \"s\"\n  $n = 1\n  Write-Host $x\n}\n",
            &[
                K::Comment,
                K::String,
                K::Number,
                K::Keyword,
                K::Function,
                K::PunctuationBracket,
            ],
        ),
        (
            DiffSyntaxLanguage::Zig,
            "// c\npub fn main() void {\n    const x = \"s\";\n    const n = 1;\n    f(x);\n}\n",
            &[
                K::Comment,
                K::String,
                K::Number,
                K::Keyword,
                K::Function,
                K::PunctuationBracket,
            ],
        ),
        (
            DiffSyntaxLanguage::Nix,
            "# c\n{\n  x = \"s\";\n  n = 1;\n}\n",
            &[K::Comment, K::String, K::Number, K::PunctuationBracket],
        ),
        (
            DiffSyntaxLanguage::Hcl,
            "# c\nresource \"a\" \"b\" {\n  n = 1\n  s = \"x\"\n  v = f(\"y\")\n}\n",
            &[
                K::Comment,
                K::String,
                K::Number,
                K::Keyword,
                K::Function,
                K::PunctuationBracket,
            ],
        ),
        (
            DiffSyntaxLanguage::Bicep,
            "// c\nparam name string = 's'\nvar n = 1\n",
            &[K::Comment, K::String, K::Number, K::Keyword],
        ),
        (
            DiffSyntaxLanguage::Sql,
            "-- c\nSELECT id, name FROM t WHERE n = 1 AND s = 'x';\n",
            &[K::Comment, K::String, K::Number, K::Keyword],
        ),
        (
            DiffSyntaxLanguage::R,
            "# c\nmain <- function() {\n  x <- \"s\"\n  n <- 1\n  f(x)\n}\n",
            &[
                K::Comment,
                K::String,
                K::Number,
                K::Function,
                K::PunctuationBracket,
            ],
        ),
        (
            DiffSyntaxLanguage::Julia,
            "# c\nfunction main()\n    x = \"s\"\n    n = 1\n    f(x)\nend\n",
            &[
                K::Comment,
                K::String,
                K::Number,
                K::Keyword,
                K::PunctuationBracket,
            ],
        ),
        (
            DiffSyntaxLanguage::Haskell,
            "-- c\nmain :: IO ()\nmain = do\n  let x = \"s\"\n  let n = 1\n  f x\n",
            &[K::Comment, K::String, K::Number, K::Keyword],
        ),
        (
            DiffSyntaxLanguage::Elixir,
            "# c\ndefmodule Foo do\n  def bar do\n    x = \"s\"\n    n = 1\n    f(x)\n  end\nend\n",
            &[
                K::Comment,
                K::String,
                K::Number,
                K::Keyword,
                K::PunctuationBracket,
            ],
        ),
        (
            DiffSyntaxLanguage::Erlang,
            "% c\n-module(foo).\nbar() ->\n    X = \"s\",\n    N = 1,\n    f(X).\n",
            &[K::Comment, K::String, K::Number, K::PunctuationBracket],
        ),
        (
            DiffSyntaxLanguage::OCaml,
            "(* c *)\nlet main () =\n  let x = \"s\" in\n  let n = 1 in\n  f x\n",
            &[K::Comment, K::String, K::Number, K::Keyword],
        ),
        (
            DiffSyntaxLanguage::FSharp,
            "// c\nlet main () =\n  let x = \"s\"\n  let n = 1\n  f x\n",
            &[K::Comment, K::String, K::Number, K::Keyword],
        ),
        (
            DiffSyntaxLanguage::Clojure,
            ";; c\n(defn main []\n  (let [x \"s\" n 1]\n    (f x)))\n",
            &[K::Comment, K::String, K::Number, K::PunctuationBracket],
        ),
        (
            DiffSyntaxLanguage::Solidity,
            "// c\ncontract Foo {\n  uint n = 1;\n  function bar() public { f(\"s\"); }\n}\n",
            &[
                K::Comment,
                K::String,
                K::Number,
                K::Keyword,
                K::PunctuationBracket,
            ],
        ),
        (
            DiffSyntaxLanguage::Assembly,
            "; c\n.section .text\nmain:\n    mov $1, %eax\n    ret\n",
            // Instructions are `@function` and labels `@label` in this
            // grammar; there is no separate keyword class to ask for.
            &[K::Comment, K::Number, K::Function, K::Label],
        ),
        (
            DiffSyntaxLanguage::Makefile,
            "# c\nOUT := a\nall: build\n\techo \"s\"\n",
            // `Function` is asserted twice over: the target name `all`, which
            // upstream's query captures only for the ~25 conventional target
            // names (see queries/makefile_supplement.scm), and `echo` inside
            // the recipe, which reaches bash through
            // queries/makefile_injections.scm. A recipe line used to be opaque
            // shell text that nothing coloured at all.
            &[
                K::Comment,
                K::Constant,
                K::Function,
                K::PunctuationDelimiter,
                K::String,
            ],
        ),
        (
            DiffSyntaxLanguage::Cmake,
            "# c\nset(NAME \"demo\")\nif(NAME)\n  add_library(y STATIC a.c)\nendif()\n",
            &[K::Comment, K::String, K::Function, K::PunctuationBracket],
        ),
        (
            DiffSyntaxLanguage::Dockerfile,
            "# c\nFROM alpine:3.20 AS build\nENV A=1\nCMD [\"app\", \"--serve\"]\n",
            // Exec form, not `RUN echo "s"`: the grammar hands a shell-form
            // command over as one opaque `shell_command`, so the quotes in it
            // are not the grammar's strings. The JSON-ish exec form is.
            &[K::Comment, K::String, K::Keyword],
        ),
        (
            DiffSyntaxLanguage::Ini,
            "; c\n[section]\nkey = value\nnumber = 42\n",
            &[K::Comment, K::Property],
        ),
        (
            DiffSyntaxLanguage::Llvm,
            "; c\ndefine i32 @main() {\n  ret i32 0\n}\n",
            // `Constant`, not `Number`: upstream's query captures integer
            // literals as `@constant`, which in LLVM IR is what they are --
            // `i32 0` is a constant operand, not a numeric literal in an
            // expression. Left as upstream has it rather than supplemented.
            &[K::Comment, K::Constant, K::Keyword],
        ),
        (
            DiffSyntaxLanguage::Just,
            "# c\nname := \"demo\"\n\nbuild:\n    echo \"hi\"\n",
            // `Function` comes from bash inside the recipe, via
            // queries/just_injections.scm -- a justfile is mostly recipes,
            // and without the injection they are plain text.
            &[K::Comment, K::String, K::Function],
        ),
        (
            DiffSyntaxLanguage::Caddyfile,
            "# c\nexample.com {\n\troot * /srv\n\tfile_server\n}\n",
            &[K::Comment, K::Keyword, K::PunctuationBracket],
        ),
        (
            DiffSyntaxLanguage::Gitignore,
            "# c\n*.log\n!keep.log\nbuild/\n",
            // The literal path text is deliberately uncoloured; what is
            // captured is what makes a line more than a path. See
            // queries/gitignore_highlights.scm. `KeywordControl` is the
            // leading `!`, which inverts the rule and is deliberately not the
            // punctuation grey `@operator` resolves to.
            &[K::Comment, K::StringRegex, K::KeywordControl],
        ),
        (
            DiffSyntaxLanguage::Crontab,
            "# c\nMAILTO=ops@example.com\n*/5 9-17 * * MON echo hi\n",
            &[K::Comment, K::Property, K::Number, K::PunctuationSpecial],
        ),
        (
            DiffSyntaxLanguage::Wat,
            ";; c\n(module\n  (func $add (param $a i32) (result i32)\n    local.get $a))\n",
            &[
                K::Comment,
                K::Keyword,
                K::TypeBuiltin,
                K::PunctuationBracket,
            ],
        ),
        (
            DiffSyntaxLanguage::Spirv,
            "; c\n%1 = OpTypeVoid\n%2 = OpConstant %int 42\n",
            &[K::Comment, K::Type, K::Operator],
        ),
        (
            DiffSyntaxLanguage::Cil,
            "// c\n.method public static void Main() cil managed\n{\n  IL_0000:  ldstr \"hi\"\n}\n",
            &[K::Comment, K::Preproc, K::Label, K::String, K::Keyword],
        ),
        (
            DiffSyntaxLanguage::Dhall,
            "-- c\nlet name = \"demo\"\nin  { greeting = name }\n",
            &[K::Comment, K::String, K::Keyword],
        ),
        (
            DiffSyntaxLanguage::CoffeeScript,
            "# c\nsquare = (x) -> x * x\nnames = ['a', 'b']\n",
            &[K::Comment, K::String],
        ),
        (
            DiffSyntaxLanguage::Perl,
            "# c\nsub greet {\n    my ($name) = @_;\n    print \"hi $name\";\n}\n",
            &[
                K::Comment,
                K::String,
                K::Keyword,
                K::Function,
                K::PunctuationBracket,
            ],
        ),
        (
            DiffSyntaxLanguage::Csv,
            "name,count\nada,1\nbob,2\n",
            // A CSV has no comments, keywords or strings -- only fields and
            // the delimiters between them.
            &[K::PunctuationDelimiter],
        ),
        (
            DiffSyntaxLanguage::Kdl,
            "// c\nnode \"arg\" key=1 {\n  child\n}\n",
            &[K::Comment, K::String, K::Number],
        ),
        (
            DiffSyntaxLanguage::Ron,
            "// c\nConfig(\n  name: \"demo\",\n  count: 1,\n)\n",
            &[K::Comment, K::String, K::Number],
        ),
        (
            DiffSyntaxLanguage::Cue,
            "// c\npackage config\n\nname: string\ncount: 1\n",
            &[K::Comment, K::Keyword, K::Number],
        ),
        (
            DiffSyntaxLanguage::Ebnf,
            "(* c *)\ndigit = \"0\" | \"1\" ;\n",
            &[K::Comment, K::String],
        ),
        (
            DiffSyntaxLanguage::Gleam,
            "// c\npub fn add(a: Int) -> Int {\n  a + 1\n}\n",
            &[K::Comment, K::Keyword, K::PunctuationBracket],
        ),
        (
            DiffSyntaxLanguage::V,
            "// c\nfn add(a int) int {\n\treturn a + 1\n}\n",
            &[K::Comment, K::Keyword, K::PunctuationBracket],
        ),
        (
            DiffSyntaxLanguage::Jsonnet,
            "// c\n{\n  name: 'demo',\n  n: 1,\n}\n",
            &[K::Comment, K::String, K::Number],
        ),
        (
            DiffSyntaxLanguage::JavaProperties,
            "# c\napp.name = demo\napp.port = 8080\n",
            &[K::Comment, K::Property],
        ),
        (
            DiffSyntaxLanguage::Proto,
            "// c\nsyntax = \"proto3\";\nmessage User {\n  string name = 1;\n}\n",
            &[K::Comment, K::String, K::Keyword, K::Type],
        ),
        (
            DiffSyntaxLanguage::Pascal,
            "unit Demo;\ninterface\n// c\nfunction Add(A: Integer): Integer;\nimplementation\nend.\n",
            &[K::Comment, K::Keyword, K::Function],
        ),
        (
            DiffSyntaxLanguage::Css,
            "/* c */\n.a { color: red; width: 1px; }\n",
            &[K::Comment, K::Number, K::Property, K::PunctuationBracket],
        ),
        (
            DiffSyntaxLanguage::Html,
            "<!-- c -->\n<div id=\"x\">t</div>\n",
            &[K::Comment, K::String, K::Tag, K::PunctuationBracket],
        ),
        (
            DiffSyntaxLanguage::Xml,
            "<!-- c -->\n<root a=\"x\">t</root>\n",
            &[K::Comment, K::String, K::Tag, K::PunctuationBracket],
        ),
        (
            DiffSyntaxLanguage::Vue,
            "<template>\n  <div id=\"x\">t</div>\n</template>\n",
            &[K::String, K::Tag, K::PunctuationBracket],
        ),
        (
            DiffSyntaxLanguage::Svelte,
            "<script>\n  let n = 1;\n</script>\n<div id=\"x\">t</div>\n",
            &[K::Tag, K::String, K::PunctuationBracket],
        ),
        (
            DiffSyntaxLanguage::Json,
            "{\n  \"a\": \"s\",\n  \"n\": 1,\n  \"b\": true\n}\n",
            &[
                K::String,
                K::Number,
                K::Property,
                K::PunctuationBracket,
                K::Boolean,
            ],
        ),
        (
            DiffSyntaxLanguage::Yaml,
            "# c\na: \"s\"\nn: 1\nb: true\n",
            &[K::Comment, K::String, K::Number, K::Property],
        ),
        (
            DiffSyntaxLanguage::Toml,
            "# c\n[t]\na = \"s\"\nn = 1\n",
            &[K::Comment, K::String, K::Number, K::Property],
        ),
        (
            DiffSyntaxLanguage::GoMod,
            "// c\nmodule example.com/m\n\ngo 1.22\n",
            &[K::Comment, K::Keyword],
        ),
        (
            DiffSyntaxLanguage::Markdown,
            "# Title\n\nSome *text* and [a link](http://x).\n",
            &[K::MarkupHeading],
        ),
    ]
};

/// Every token kind a sample comes out coloured with.
///
/// The one definition of "what this language emits", so the per-language
/// tests below and the baseline sweep cannot disagree about what counts.
fn token_kinds_in_sample(language: DiffSyntaxLanguage, text: &str) -> Vec<SyntaxTokenKind> {
    let document = prepare_test_document(language, text);
    let mut seen: Vec<SyntaxTokenKind> = Vec::new();
    for ix in 0..text.lines().count() {
        if let Some(chunk) = syntax_tokens_for_prepared_document_line(document, ix) {
            seen.extend(chunk.iter().map(|token| token.kind));
        }
    }
    seen
}

/// The kinds a language must be able to colour before it counts as wired.
///
/// Not a style preference: each of these is something a reader looks for. A
/// grammar whose query cannot emit `Comment` leaves comments the same colour
/// as code; one with no `PunctuationBracket` leaves every brace flat.
fn assert_language_colours(language: DiffSyntaxLanguage, text: &str, required: &[SyntaxTokenKind]) {
    let seen = token_kinds_in_sample(language, text);
    for kind in required {
        assert!(
            seen.contains(kind),
            "{language:?} never produced {kind:?}; it emitted {seen:?}"
        );
    }
}

/// Every language in [`LANGUAGE_BASELINES`] colours what it must.
///
/// Reports every gap in one run rather than stopping at the first, so a
/// grammar batch can be assessed in one go.
#[test]
fn every_wired_language_meets_its_highlight_baseline() {
    let mut gaps: Vec<String> = Vec::new();
    for (language, sample, required) in LANGUAGE_BASELINES {
        let mut seen = token_kinds_in_sample(*language, sample);
        let missing: Vec<_> = required
            .iter()
            .filter(|kind| !seen.contains(kind))
            .collect();
        if !missing.is_empty() {
            seen.sort_by_key(|kind| format!("{kind:?}"));
            seen.dedup();
            gaps.push(format!("{language:?} missing {missing:?} (has {seen:?})"));
        }
    }
    assert!(gaps.is_empty(), "highlight gaps:\n  {}", gaps.join("\n  "));
}

/// Objective-C's own query captures neither comments, strings nor numbers,
/// so a `.m` file used to render those as plain code.
#[test]
fn objective_c_colours_comments_strings_and_numbers() {
    assert_language_colours(
        DiffSyntaxLanguage::ObjectiveC,
        "// a comment\n@implementation Foo\n- (void)bar {\n  NSString *s = @\"hi\";\n  int n = 42;\n}\n@end\n",
        &[
            SyntaxTokenKind::Comment,
            SyntaxTokenKind::String,
            SyntaxTokenKind::Number,
            SyntaxTokenKind::Type,
            SyntaxTokenKind::PunctuationBracket,
        ],
    );
}

/// Java, PHP and Groovy all use queries that capture no punctuation at all.
#[test]
fn jvm_and_php_family_colour_their_brackets() {
    assert_language_colours(
        DiffSyntaxLanguage::Java,
        "// c\nclass Foo {\n  int count = 1;\n  void bar() {\n    this.count = other.value;\n  }\n}\n",
        &[
            SyntaxTokenKind::Comment,
            SyntaxTokenKind::PunctuationBracket,
            SyntaxTokenKind::PunctuationDelimiter,
            SyntaxTokenKind::Property,
        ],
    );
    assert_language_colours(
        DiffSyntaxLanguage::Php,
        "<?php\n// c\nclass Foo {\n  public $count = 1;\n  function bar() { return $this->count; }\n}\n",
        &[
            SyntaxTokenKind::Comment,
            SyntaxTokenKind::PunctuationBracket,
            SyntaxTokenKind::PunctuationDelimiter,
            SyntaxTokenKind::Property,
        ],
    );
    assert_language_colours(
        DiffSyntaxLanguage::Groovy,
        "// c\nclass Foo {\n  int count = 1\n  def bar() { return this.count }\n}\n",
        &[
            SyntaxTokenKind::Comment,
            SyntaxTokenKind::PunctuationBracket,
            SyntaxTokenKind::Keyword,
        ],
    );
}

/// PowerShell's query tags `(array_expression)` `@array`, which spans the
/// whole `@(1, 2)` -- parens, commas and the spaces between. Mapping that to
/// a type made an array literal read as one long type name.
#[test]
fn powershell_array_literals_are_not_one_long_type() {
    let text = "# c\nfunction Get-Thing {\n  $x = @(1, 2)\n  return $x.Count\n}\n";
    let document = prepare_test_document(DiffSyntaxLanguage::PowerShell, text);
    let line = text.lines().nth(2).expect("array line");
    let tokens = syntax_tokens_for_prepared_document_line(document, 2)
        .map(|chunk| chunk.to_vec())
        .unwrap_or_default();
    let rendered: Vec<(&str, SyntaxTokenKind)> = tokens
        .iter()
        .map(|token| (&line[token.range.clone()], token.kind))
        .collect();
    assert!(
        !rendered
            .iter()
            .any(|(_, kind)| *kind == SyntaxTokenKind::Type),
        "nothing on an array literal line is a type, got {rendered:?}"
    );
    assert!(
        rendered.contains(&("@(", SyntaxTokenKind::PunctuationBracket))
            && rendered.contains(&(")", SyntaxTokenKind::PunctuationBracket)),
        "the array's own brackets should be brackets, got {rendered:?}"
    );
    assert!(
        rendered.contains(&("1", SyntaxTokenKind::Number)),
        "its elements keep their own colours, got {rendered:?}"
    );
}

/// Terraform gets real syntax, and with it the tree that delimiter matching
/// and name highlighting need.
///
/// `tree-sitter-hcl` ships no highlights query, so `.tf` was heuristic-only
/// and had no document tree at all: no bracket pairs, no occurrences.
/// `queries/hcl_highlights.scm` is authored in-tree for exactly that.
#[test]
fn terraform_files_get_tree_sitter_tokens() {
    let text = concat!(
        "# managed by terraform\n",
        "resource \"aws_instance\" \"web\" {\n",
        "  ami           = var.ami_id\n",
        "  count         = 2\n",
        "  enabled       = true\n",
        "  name          = \"web-${var.env}\"\n",
        "  user_data     = file(\"init.sh\")\n",
        "}\n",
    );
    assert_eq!(
        diff_syntax_language_for_path(std::path::Path::new("main.tf")),
        Some(DiffSyntaxLanguage::Hcl)
    );
    let document = prepare_test_document(DiffSyntaxLanguage::Hcl, text);
    let kinds = |ix: usize| -> Vec<(String, SyntaxTokenKind)> {
        let line = text.lines().nth(ix).expect("line");
        syntax_tokens_for_prepared_document_line(document, ix)
            .map(|chunk| {
                chunk
                    .iter()
                    .map(|t| (line[t.range.clone()].to_string(), t.kind))
                    .collect()
            })
            .unwrap_or_default()
    };

    assert_eq!(
        kinds(0),
        vec![("# managed by terraform".into(), SyntaxTokenKind::Comment)]
    );
    let header = kinds(1);
    assert!(
        header.contains(&("resource".into(), SyntaxTokenKind::Keyword))
            && header.contains(&("\"aws_instance\"".into(), SyntaxTokenKind::String)),
        "block header should name its type and labels, got {header:?}"
    );
    assert!(
        kinds(3).contains(&("2".into(), SyntaxTokenKind::Number)),
        "numbers should be numbers, got {:?}",
        kinds(3)
    );
    assert!(
        kinds(4).contains(&("true".into(), SyntaxTokenKind::Boolean)),
        "booleans should be booleans, got {:?}",
        kinds(4)
    );
    // An interpolated string keeps its literal halves coloured around `${}`.
    let interpolated = kinds(5);
    assert!(
        interpolated.contains(&("\"web-".into(), SyntaxTokenKind::String))
            && interpolated.contains(&("${".into(), SyntaxTokenKind::PunctuationSpecial))
            && interpolated.contains(&("var".into(), SyntaxTokenKind::Variable)),
        "interpolation should split string from code, got {interpolated:?}"
    );
    assert!(
        kinds(6).contains(&("file".into(), SyntaxTokenKind::Function)),
        "calls should be functions, got {:?}",
        kinds(6)
    );
}

/// And with a tree in hand, both click features work in Terraform.
#[test]
fn terraform_supports_pairs_and_occurrences() {
    let text = concat!(
        "resource \"aws_instance\" \"web\" {\n",
        "  ami   = var.ami_id\n",
        "  other = var.ami_id\n",
        "}\n",
    );
    let document = prepare_test_document(DiffSyntaxLanguage::Hcl, text);

    // The block braces pair across the whole resource.
    let hit = prepared_document_syntax_pair_at_display_offset(document, 1, 4)
        .expect("the resource block braces should pair");
    assert_eq!(hit.kind, SyntaxPairKind::Bracket);
    assert_eq!(hit.open[0].line_ix, 0);
    assert_eq!(hit.close[0].line_ix, 3);

    // And `ami_id` is a name with two uses.
    let column = text
        .lines()
        .nth(1)
        .expect("line")
        .find("ami_id")
        .expect("name");
    let spans = prepared_document_occurrences_at_display_offset(document, 1, column);
    assert_eq!(
        spans.iter().map(|span| span.line_ix).collect::<Vec<_>>(),
        vec![1, 2]
    );
}

/// The Z80 shadow-register apostrophe does not open a string, on either path.
///
/// `AF'`, `BC'`, `DE'`, `HL'` end in an apostrophe in a language that also
/// spells character literals with one, so `EX AF, AF'` used to open a string
/// that ran to the next quote fifty lines later. The grammar half is
/// `shadow_reg` in vendor/tree-sitter-asm; this covers the heuristic, which is
/// what the app renders whenever the parse budget blows -- so a fix to only
/// one of the two is invisible in whichever regime you happen to test.
#[test]
fn assembly_shadow_register_apostrophe_does_not_open_a_string() {
    let heuristic = |line: &str| {
        syntax_tokens_for_line(
            line,
            DiffSyntaxLanguage::Assembly,
            DiffSyntaxMode::HeuristicOnly,
        )
        .to_vec()
    };

    let shadow = heuristic("        EX   AF, AF'");
    assert!(
        shadow.iter().all(|t| t.kind != SyntaxTokenKind::String),
        "a shadow register is not a string: {shadow:?}"
    );

    // ...but a real character literal still is, and the two differ only by
    // what precedes the quote.
    let literal = heuristic("CHAR    EQU 'A'");
    assert!(
        literal.iter().any(|t| t.kind == SyntaxTokenKind::String),
        "`'A'` in value position is still a char literal: {literal:?}"
    );

    // And the tree-sitter path agrees, which is the point of testing both.
    let text = concat!(
        "        EX   AF, AF'\n",
        "        LD   A, 'X'\n",
        "        HALT\n",
    );
    let document = prepare_test_document(DiffSyntaxLanguage::Assembly, text);
    let kinds = |line_ix: usize| -> Vec<SyntaxTokenKind> {
        syntax_tokens_for_prepared_document_line(document, line_ix)
            .map(|tokens| tokens.iter().map(|token| token.kind).collect())
            .unwrap_or_default()
    };
    assert!(
        !kinds(0).contains(&SyntaxTokenKind::String),
        "grammar: `AF'` is a register, got {:?}",
        kinds(0)
    );
    assert!(
        kinds(1).contains(&SyntaxTokenKind::String),
        "grammar: `'X'` is a char literal, got {:?}",
        kinds(1)
    );
    assert_eq!(
        kinds(2),
        vec![SyntaxTokenKind::Function],
        "the line after must not have been swallowed by a string"
    );
}

/// A GAS local label reference is a label, not a register.
///
/// `1f`/`2b` are the jump targets in hand-written x86 assembly, and the
/// grammar files them as `reg` like every other bare operand -- so `je 2f`
/// painted its target the same colour as `%r13`. The register assertions are
/// the other half: matching by shape rather than by branch mnemonic is what
/// keeps ARM's `bic` and x86's `bswap` from having their operands relabelled.
#[test]
fn assembly_local_label_references_are_labels_not_registers() {
    let text = concat!(
        "1:      cmpq    $0, %r13\n",
        "        je      2f\n",
        "        jmp     1b\n",
        "        bswap   %eax\n",
        "2:\n",
    );
    let document = prepare_test_document(DiffSyntaxLanguage::Assembly, text);
    let kind_of = |line_ix: usize, needle: &str| -> Option<SyntaxTokenKind> {
        let line = text.lines().nth(line_ix)?;
        let at = line.find(needle)?;
        syntax_tokens_for_prepared_document_line(document, line_ix)?
            .iter()
            .find(|token| token.range.start == at)
            .map(|token| token.kind)
    };

    assert_eq!(kind_of(1, "2f"), Some(SyntaxTokenKind::Label));
    assert_eq!(kind_of(2, "1b"), Some(SyntaxTokenKind::Label));
    // The definitions they point at.
    assert_eq!(kind_of(0, "1:"), Some(SyntaxTokenKind::Label));
    assert_eq!(kind_of(4, "2:"), Some(SyntaxTokenKind::Label));
    // And nothing else moved: registers are still registers, including on a
    // mnemonic that starts with `b` but is not a branch.
    assert_eq!(
        kind_of(0, "%r13"),
        Some(SyntaxTokenKind::VariableBuiltin),
        "a `%` register is not a label"
    );
    assert_eq!(
        kind_of(3, "%eax"),
        Some(SyntaxTokenKind::VariableBuiltin),
        "`bswap` is not a branch, and its operand is a register"
    );
}

/// Clicking an assembler directive or a dotted mnemonic finds the others.
///
/// Both were dead before: `is_name` in syntax/occurrences.rs required a
/// name's first character to be a letter or `_` and every character after it
/// to be alphanumeric, so `.section` and `.p2align` never got past it, and
/// `b.eq` was not one node to click in the first place (see
/// vendor/tree-sitter-asm). An assembly file is mostly these two things, so
/// between them the feature did nothing at all on `.s`.
#[test]
fn assembly_directives_and_dotted_mnemonics_answer_a_click() {
    let text = concat!(
        ".section .text\n",
        "    .p2align 4\n",
        "main:\n",
        "    b.eq main\n",
        "    b.ne main\n",
        "    b.eq main\n",
        ".section .data\n",
    );
    let document = prepare_test_document(DiffSyntaxLanguage::Assembly, text);

    // A leading `.` is the whole point: the directive is the name.
    let spans = prepared_document_occurrences_at_display_offset(document, 0, 3);
    assert_eq!(
        spans.iter().map(|span| span.line_ix).collect::<Vec<_>>(),
        vec![0, 6],
        "clicking `.section` should find both of them"
    );

    // And the dot is interior here, in a token the vendored grammar keeps
    // whole. `b.ne` on line 4 must not match.
    let spans = prepared_document_occurrences_at_display_offset(document, 3, 6);
    assert_eq!(
        spans.iter().map(|span| span.line_ix).collect::<Vec<_>>(),
        vec![3, 5],
        "clicking `b.eq` should find the other `b.eq` and not `b.ne`"
    );
}

/// Perl has a tree, so brackets pair and names answer a click.
///
/// It was heuristic-only before, and the report was that "the bracket and word
/// highlight is broken" -- they were not broken, they were absent. Both read
/// the tree, not the tokens, so a language with no grammar has neither however
/// well the heuristic colours it.
#[test]
fn perl_brackets_and_names_answer_a_click() {
    let text = "my %hash = (one => 1);\nmy $v = $hash{one};\nmy $w = $hash{two};\n";
    let document = prepare_test_document(DiffSyntaxLanguage::Perl, text);
    for line_ix in 0..3 {
        let _ = syntax_tokens_for_prepared_document_line(document, line_ix);
    }

    let line = text.lines().nth(1).expect("line");
    let brace = line.find('{').expect("subscript brace");
    let hit = prepared_document_syntax_pair_at_display_offset(document, 1, brace)
        .expect("a hash subscript's braces should pair");
    assert_eq!(hit.kind, SyntaxPairKind::Bracket);
    assert_eq!(
        hit.close.first().map(|span| span.display_range.start),
        Some(line.find('}').expect("closing brace"))
    );

    // ...and `hash` is a name the grammar tokenised, so a click finds its uses.
    let at = line.find("hash").expect("name");
    let spans = prepared_document_occurrences_at_display_offset(document, 1, at + 1);
    assert_eq!(
        spans.iter().map(|span| span.line_ix).collect::<Vec<_>>(),
        vec![1, 2],
        "clicking `$hash` should find the other `$hash`"
    );
    // ...and not the `%hash` on line 0. Perl writes the same hash with a
    // different sigil depending on what is taken from it, and the sigil is
    // part of the token, so those are different text. Connecting them would
    // need to know the language.
}

/// A CSS custom property takes any balanced tokens, and does not derail the
/// rest of the block.
///
/// `--json: {...}` used to parse as a nested *rule set* -- CSS nesting makes
/// `name: {` a selector with a pseudo-class colon -- and the ERROR took every
/// later declaration with it. See vendor/tree-sitter-css.
#[test]
fn css_custom_property_does_not_derail_the_block() {
    let text = concat!(
        ":root {\n",
        "  --brand: #3366ff;\n",
        "  --json: {\"key\": \"value\"};\n",
        "  color: red;\n",
        "}\n",
    );
    let document = prepare_test_document(DiffSyntaxLanguage::Css, text);
    let kinds = |line_ix: usize| -> Vec<SyntaxTokenKind> {
        syntax_tokens_for_prepared_document_line(document, line_ix)
            .map(|tokens| tokens.iter().map(|token| token.kind).collect())
            .unwrap_or_default()
    };

    // An ordinary custom property is untouched: the scanner declines unless
    // the value has a brace, so this keeps the tree it always had.
    assert!(
        kinds(1).contains(&SyntaxTokenKind::Variable),
        "`--brand` is a custom property, got {:?}",
        kinds(1)
    );
    // ...and the declaration after the braced one still reads as a property,
    // which is the whole point -- it used to be swallowed by the ERROR.
    assert!(
        kinds(3).contains(&SyntaxTokenKind::Property),
        "`color` after a braced custom property, got {:?}",
        kinds(3)
    );
    assert!(
        kinds(3).contains(&SyntaxTokenKind::ConstantBuiltin),
        "`red` should still be a value, got {:?}",
        kinds(3)
    );
}

/// A Ruby `=begin` block ends only at an `=end` in column 0.
///
/// Upstream matched the body a character at a time and stopped at any `=end`,
/// so a block comment that *mentions* `=end` -- prose about Ruby's own
/// comment syntax, which is where it appears -- ended early and the rest
/// parsed as code. See vendor/tree-sitter-ruby.
#[test]
fn ruby_block_comment_ends_only_at_column_zero() {
    let text = concat!(
        "=begin\n",
        "both =begin and =end must start at column 0\n",
        "  def never_defined\n",
        "    \"an unterminated string is fine in here: 'nope\n",
        "=end\n",
        "real_code = 1\n",
    );
    let document = prepare_test_document(DiffSyntaxLanguage::Ruby, text);
    let kinds = |line_ix: usize| -> Vec<SyntaxTokenKind> {
        syntax_tokens_for_prepared_document_line(document, line_ix)
            .map(|tokens| tokens.iter().map(|token| token.kind).collect())
            .unwrap_or_default()
    };

    for line_ix in 0..=4 {
        assert_eq!(
            kinds(line_ix),
            vec![SyntaxTokenKind::Comment],
            "line {line_ix} is inside the block comment"
        );
    }
    assert!(
        kinds(5).contains(&SyntaxTokenKind::Variable),
        "code resumes after `=end`, got {:?}",
        kinds(5)
    );
}

/// `</script` inside a string or comment does not end the element.
///
/// The spec closes a script only when the tag name is followed by whitespace,
/// `/` or `>`. Upstream's scanner broke on the characters alone, so a script
/// that *mentions* `</script` -- a bundler note, a regex, code about HTML --
/// ended there and the rest of it rendered as markup. See
/// vendor/tree-sitter-html.
#[test]
fn html_script_survives_a_close_tag_inside_a_string() {
    let text = concat!(
        "<script>\n",
        "  // the characters \"</script\" do not end this\n",
        "  const re = /<\\/script>/;\n",
        "  const done = 1;\n",
        "</script>\n",
    );
    let document = prepare_test_document(DiffSyntaxLanguage::Html, text);
    let kinds = |line_ix: usize| -> Vec<SyntaxTokenKind> {
        syntax_tokens_for_prepared_document_line(document, line_ix)
            .map(|tokens| tokens.iter().map(|token| token.kind).collect())
            .unwrap_or_default()
    };

    assert!(
        kinds(1).contains(&SyntaxTokenKind::Comment),
        "the line mentioning `</script` is still a JS comment, got {:?}",
        kinds(1)
    );
    assert!(
        kinds(3).contains(&SyntaxTokenKind::Keyword),
        "JavaScript continues after it, got {:?}",
        kinds(3)
    );
}

/// A `<script>` is JavaScript only when its type says so.
///
/// `type="text/template"` holds raw text a framework reads later, and
/// injecting JavaScript into it coloured English prose as variables and
/// operators. `application/json` is JSON and now reads as JSON.
#[test]
fn html_script_injection_follows_the_type_attribute() {
    let text = concat!(
        "<script>const a = 1;</script>\n",
        "<script type=\"text/template\" id=\"tpl\">raw text here</script>\n",
        "<script type=\"application/json\">{\"key\": 1}</script>\n",
    );
    let document = prepare_test_document(DiffSyntaxLanguage::Html, text);
    let kinds = |line_ix: usize| -> Vec<SyntaxTokenKind> {
        syntax_tokens_for_prepared_document_line(document, line_ix)
            .map(|tokens| tokens.iter().map(|token| token.kind).collect())
            .unwrap_or_default()
    };

    assert!(
        kinds(0).contains(&SyntaxTokenKind::Keyword),
        "an untyped script is still JavaScript, got {:?}",
        kinds(0)
    );
    assert!(
        !kinds(1).contains(&SyntaxTokenKind::Keyword)
            && !kinds(1).contains(&SyntaxTokenKind::Variable),
        "a template script's body is raw text, got {:?}",
        kinds(1)
    );
    assert!(
        kinds(2).contains(&SyntaxTokenKind::Property),
        "`\"key\"` should be a JSON key, got {:?}",
        kinds(2)
    );
}

/// A broad capture whose only predicate is one the engine ignores.
///
/// `#is-not? local` (Ruby) and the unscoped `(variable) @type` (Haskell) are
/// the same failure as `#lua-match?`: the rule does not filter, it sits below
/// the general rule it was meant to refine, and last-wins makes it the answer
/// for the whole file. Ruby rendered every local variable in the method
/// colour; Haskell rendered every lower-case identifier as a type.
///
/// The JavaScript query keeps its own `#is-not? local` on purpose -- see
/// queries/ruby_highlights.scm's header for why that one is harmless.
#[test]
fn unevaluated_predicates_do_not_swallow_whole_files() {
    let ruby = "plain = 1\nputs plain\n";
    let document = prepare_test_document(DiffSyntaxLanguage::Ruby, ruby);
    let kinds = |line_ix: usize| -> Vec<SyntaxTokenKind> {
        syntax_tokens_for_prepared_document_line(document, line_ix)
            .map(|tokens| tokens.iter().map(|token| token.kind).collect())
            .unwrap_or_default()
    };
    assert!(
        kinds(0).contains(&SyntaxTokenKind::Variable),
        "a local binding is a variable, got {:?}",
        kinds(0)
    );
    assert!(
        kinds(1).contains(&SyntaxTokenKind::FunctionMethod),
        "`puts` is still a method call, got {:?}",
        kinds(1)
    );

    let haskell = "add :: Int -> Int -> Int\nadd a b = a + b\n";
    let document = prepare_test_document(DiffSyntaxLanguage::Haskell, haskell);
    let kinds = |line_ix: usize| -> Vec<SyntaxTokenKind> {
        syntax_tokens_for_prepared_document_line(document, line_ix)
            .map(|tokens| tokens.iter().map(|token| token.kind).collect())
            .unwrap_or_default()
    };
    assert!(
        kinds(0).contains(&SyntaxTokenKind::Type),
        "`Int` is a type, got {:?}",
        kinds(0)
    );
    assert!(
        !kinds(1).contains(&SyntaxTokenKind::Type),
        "`a` and `b` are values, not types: {:?}",
        kinds(1)
    );
}

/// A click in an injected region is answered by the injected grammar.
///
/// `prepared_document_syntax_pair_at_display_offset` had only the host tree,
/// so in an injected region there was no structure to pair against at all --
/// to PHP, a file's whole inline HTML is one `text` node. That is what made a
/// bracket click appear to do nothing there while a click a column away
/// appeared to work: the column away fell through to the enclosing host
/// construct, which did answer.
///
/// The row has to be drawn first, exactly as in the app: drawing is what
/// parses the injection and puts its tree in the cache.
#[test]
fn a_click_in_an_injected_region_uses_the_injected_grammar() {
    let text = concat!("<?php f($t); ?>\n", "<html lang=\"en\">\n");
    let document = prepare_test_document(DiffSyntaxLanguage::Php, text);
    let _ = syntax_tokens_for_prepared_document_line(document, 1);

    let line = text.lines().nth(1).expect("line");
    let quote = line.find('"').expect("attribute quote");
    let hit = prepared_document_syntax_pair_at_display_offset(document, 1, quote)
        .expect("the attribute's quotes are HTML's to pair, not PHP's");
    assert_eq!(hit.kind, SyntaxPairKind::Quote);
    assert_eq!(hit.open.first().map(|span| span.line_ix), Some(1));
    assert_eq!(hit.close.first().map(|span| span.line_ix), Some(1));

    // ...and the host grammar still answers where it owns the bytes.
    let _ = syntax_tokens_for_prepared_document_line(document, 0);
    let php = text.lines().next().expect("line");
    let paren = php.find('(').expect("php call");
    let hit = prepared_document_syntax_pair_at_display_offset(document, 0, paren)
        .expect("the call's parens are PHP's");
    assert_eq!(hit.kind, SyntaxPairKind::Bracket);
    assert_eq!(
        hit.close.first().map(|span| span.display_range.start),
        Some(php.find(')').expect("closing paren")),
        "the host pair should be the call's own parens"
    );
}

#[test]
fn review_nested_injected_pair_uses_inner_grammar() {
    TS_INJECTION_CACHE.with(|cache| cache.borrow_mut().clear());
    let text = "<script>const x = (1);</script>\n<?php f(); ?>\n";
    let document = prepare_test_document(DiffSyntaxLanguage::Php, text);
    let _ = syntax_tokens_for_prepared_document_line(document, 0);
    let line = text.lines().next().unwrap();
    let open = line.find('(').unwrap();
    let close = line.find(')').unwrap();
    let hit = prepared_document_syntax_pair_at_display_offset(document, 0, open)
        .expect("nested JavaScript's parens should pair");
    assert_eq!(hit.kind, SyntaxPairKind::Bracket, "got {hit:?}");
    assert_eq!(hit.open[0].display_range, open..open + 1, "got {hit:?}");
    assert_eq!(hit.close[0].display_range, close..close + 1, "got {hit:?}");
}

/// The HTML around a PHP file's `<?php ?>` islands is HTML.
///
/// A PHP file is two languages interleaved and the grammar hands the inline
/// half over as one structureless `text` node, so without
/// queries/php_injections.scm every such region rendered as plain body text.
/// It is not a rare shape: a `?>` inside a `//` comment really does end PHP
/// mode, which is what puts most of the corpus's comments.php into HTML.
#[test]
fn php_inline_html_is_highlighted_as_html() {
    let text = concat!(
        "<?php $title = 'hi'; ?>\n",
        "<html lang=\"en\">\n",
        "<h1><?= $title ?></h1>\n",
    );
    let document = prepare_test_document(DiffSyntaxLanguage::Php, text);
    let kinds = |line_ix: usize| -> Vec<SyntaxTokenKind> {
        syntax_tokens_for_prepared_document_line(document, line_ix)
            .map(|tokens| tokens.iter().map(|token| token.kind).collect())
            .unwrap_or_default()
    };

    assert!(
        kinds(1).contains(&SyntaxTokenKind::Tag),
        "`<html>` should be a tag, got {:?}",
        kinds(1)
    );
    assert!(
        kinds(1).contains(&SyntaxTokenKind::Attribute),
        "`lang=` should be an attribute, got {:?}",
        kinds(1)
    );
    // ...and the PHP island inside the HTML keeps its own colours.
    assert!(
        kinds(2).contains(&SyntaxTokenKind::Variable),
        "`$title` should still be PHP, got {:?}",
        kinds(2)
    );
}

/// `.json5` resolves to JavaScript, not JSON.
///
/// Everything JSON5 adds over JSON -- comments, unquoted keys, single quotes,
/// trailing commas -- is ordinary ECMAScript object-literal syntax.
/// tree-sitter-json produces 72 error nodes in the 58-line corpus sample;
/// tree-sitter-javascript produces none.
#[test]
fn json5_resolves_to_javascript() {
    assert_eq!(
        diff_syntax_language_for_path("config/data.json5"),
        Some(DiffSyntaxLanguage::JavaScript)
    );
    let text = "{\n  // a comment\n  unquoted: 'single quotes',\n}\n";
    let document = prepare_test_document(DiffSyntaxLanguage::JavaScript, text);
    let kinds = |line_ix: usize| -> Vec<SyntaxTokenKind> {
        syntax_tokens_for_prepared_document_line(document, line_ix)
            .map(|tokens| tokens.iter().map(|token| token.kind).collect())
            .unwrap_or_default()
    };
    assert!(kinds(1).contains(&SyntaxTokenKind::Comment));
    assert!(kinds(2).contains(&SyntaxTokenKind::String));
}

/// A Terraform heredoc body highlights as what is inside it.
///
/// HCL has no syntax for declaring a heredoc's language, so `user_data` and
/// `policy` -- the two densest things in a real Terraform file -- rendered as
/// one flat `@string` for however many lines they ran. The language comes from
/// the attribute name; see queries/hcl_injections.scm for why not from the
/// content.
#[test]
fn hcl_heredoc_bodies_are_injected_by_attribute_name() {
    let text = concat!(
        "resource \"aws_instance\" \"web\" {\n",
        "  user_data = <<-EOT\n",
        "    set -euo pipefail\n",
        "    echo \"hi ${count.index}\"\n",
        "  EOT\n",
        "\n",
        "  policy = <<EOT\n",
        "{\"Version\": \"2012-10-17\"}\n",
        "EOT\n",
        "\n",
        "  description = <<EOT\n",
        "just prose, no language to guess\n",
        "EOT\n",
        "}\n",
    );
    let document = prepare_test_document(DiffSyntaxLanguage::Hcl, text);
    let kinds = |line_ix: usize| -> Vec<SyntaxTokenKind> {
        syntax_tokens_for_prepared_document_line(document, line_ix)
            .map(|tokens| tokens.iter().map(|token| token.kind).collect())
            .unwrap_or_default()
    };

    // Shell, because the attribute is `user_data`.
    assert!(
        kinds(3).contains(&SyntaxTokenKind::Function),
        "`echo` should be a command, got {:?}",
        kinds(3)
    );
    // ...and the HCL interpolation inside it keeps the HCL colours, because it
    // is not part of the injected range.
    assert!(
        kinds(3).contains(&SyntaxTokenKind::Property),
        "`${{count.index}}` should still read as Terraform, got {:?}",
        kinds(3)
    );

    // JSON, because the attribute is `policy`.
    assert!(
        kinds(7).contains(&SyntaxTokenKind::Property),
        "`\"Version\"` should be a JSON key, got {:?}",
        kinds(7)
    );

    // And an attribute the convention says nothing about is left alone rather
    // than guessed at.
    assert_eq!(
        kinds(11),
        vec![SyntaxTokenKind::String],
        "an unrecognised attribute's heredoc stays a string"
    );
}

/// Clicking an `ARG`/`ENV` name in a Dockerfile finds its other uses.
///
/// Same root cause as the YAML case below: `tree-sitter-containerfile` calls
/// the bare word on both sides of an `ARG`/`ENV` pair `unquoted_string`, and
/// `is_name_token_kind` rejects any kind containing "string". Tracing an
/// `ARG` through the stages that re-declare it is most of what reading a
/// Dockerfile is, and none of it answered a click.
#[test]
fn dockerfile_arg_names_answer_a_click() {
    let text = concat!(
        "ARG BASE_TAG=bookworm\n",
        "FROM debian:${BASE_TAG} AS builder\n",
        "ARG BASE_TAG\n",
        "ENV OTHER=1\n",
    );
    let document = prepare_test_document(DiffSyntaxLanguage::Dockerfile, text);

    let spans = prepared_document_occurrences_at_display_offset(document, 0, 6);
    assert_eq!(
        spans.iter().map(|span| span.line_ix).collect::<Vec<_>>(),
        vec![0, 1, 2],
        "the declaration, the `${{...}}` use, and the re-declaration after FROM"
    );

    // The expansion form resolves to the same set from the other end.
    let line = text.lines().nth(1).expect("line");
    let column = line.find("BASE_TAG").expect("needle");
    let spans = prepared_document_occurrences_at_display_offset(document, 1, column + 2);
    assert_eq!(
        spans.iter().map(|span| span.line_ix).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
}

/// The `!` that re-includes a path is the accent colour, not punctuation grey.
///
/// It is the one character in a `.gitignore` that reverses a line's meaning,
/// and `@operator` resolves to `foreground.secondary` -- the same muted grey
/// as the comments around it.
#[test]
fn gitignore_negation_is_prominent() {
    let text = "*.log\n!important.log\nbuild/\n";
    let document = prepare_test_document(DiffSyntaxLanguage::Gitignore, text);
    let tokens = syntax_tokens_for_prepared_document_line(document, 1)
        .map(|tokens| tokens.to_vec())
        .unwrap_or_default();
    assert_eq!(
        tokens.first().map(|token| token.kind),
        Some(SyntaxTokenKind::KeywordControl),
        "the leading `!` should read as an inversion, got {tokens:?}"
    );
}

/// Clicking a YAML mapping key finds the key's other uses.
///
/// `is_name_token_kind` rejects any node kind containing "string", which is
/// right for string *contents* and wrong for exactly one grammar:
/// tree-sitter-yaml calls the unquoted plain scalar `string_scalar`, so every
/// key in every playbook read as string content. An Ansible file is nothing
/// but keys, which is where this was reported.
#[test]
fn yaml_plain_mapping_keys_answer_a_click() {
    let text = concat!(
        "- hosts: web\n",
        "  become: true\n",
        "  tasks:\n",
        "    - name: install\n",
        "      ansible.builtin.copy:\n",
        "        src: a\n",
        "- hosts: db\n",
        "  become: false\n",
    );
    let document = prepare_test_document(DiffSyntaxLanguage::Yaml, text);

    let spans = prepared_document_occurrences_at_display_offset(document, 0, 4);
    assert_eq!(
        spans.iter().map(|span| span.line_ix).collect::<Vec<_>>(),
        vec![0, 6],
        "clicking the `hosts` key should find both plays"
    );

    // A dotted module name is one plain scalar, so it is one name.
    let spans = prepared_document_occurrences_at_display_offset(document, 4, 10);
    assert_eq!(
        spans.iter().map(|span| span.line_ix).collect::<Vec<_>>(),
        vec![4],
        "clicking `ansible.builtin.copy` should resolve to the whole key"
    );
}

/// A prose-only `/** ... */` must still read as a comment.
///
/// The jsdoc injection subtracts the host's `(comment) @comment` from the
/// document before painting its own captures, so a doc comment with no
/// `@tag` in it used to come out with no colour at all -- plain foreground
/// text in the middle of a file. The base capture in `jsdoc_highlights.scm`
/// is what puts the comment colour back.
#[test]
fn jsdoc_comments_keep_their_comment_colour() {
    let text = concat!(
        "/**\n",
        " * Where to go after signing in.\n",
        " *\n",
        " * Same-origin paths only, so no open redirect.\n",
        " */\n",
        "export function f() {}\n",
    );
    let document = prepare_test_document(DiffSyntaxLanguage::Tsx, text);
    for (ix, line) in text.lines().enumerate().take(5) {
        let tokens = syntax_tokens_for_prepared_document_line(document, ix)
            .map(|chunk| chunk.to_vec())
            .unwrap_or_default();
        assert!(
            !tokens.is_empty(),
            "line {ix} ({line:?}) of a doc comment has no tokens at all"
        );
        assert!(
            tokens
                .iter()
                .all(|token| token.kind == SyntaxTokenKind::Comment),
            "line {ix} ({line:?}) should be all comment, got {tokens:?}"
        );
    }
}

/// ...and a tagged one still gets its tag, type and name picked out of it.
#[test]
fn jsdoc_tags_still_win_over_the_comment_base() {
    let text = "/**\n * @param {string} search - the query.\n */\nlet a = 1;\n";
    let document = prepare_test_document(DiffSyntaxLanguage::Tsx, text);
    let line = text.lines().nth(1).expect("tag line");
    let tokens = syntax_tokens_for_prepared_document_line(document, 1)
        .map(|chunk| chunk.to_vec())
        .unwrap_or_default();
    let kinds: Vec<(&str, SyntaxTokenKind)> = tokens
        .iter()
        .map(|token| (&line[token.range.clone()], token.kind))
        .collect();
    assert!(
        kinds.contains(&("@param", SyntaxTokenKind::Keyword)),
        "expected the tag to stay a keyword, got {kinds:?}"
    );
    assert!(
        kinds.contains(&("string", SyntaxTokenKind::Type)),
        "expected the type to stay a type, got {kinds:?}"
    );
    assert!(
        kinds
            .iter()
            .any(|(text, kind)| *kind == SyntaxTokenKind::Comment && text.contains("query")),
        "the prose around the tag should still be comment, got {kinds:?}"
    );
}

/// A JSX expression comment spans several rows, and every one of them is
/// comment -- backticks and quotes inside it are prose, not code.
#[test]
fn jsx_expression_comments_stay_comments_on_every_line() {
    let text = concat!(
        "const a = (\n",
        "  <div>\n",
        "    {/* `exact` matters: `/app/device` is a prefix of\n",
        "    `/app/devices`, so a user clicking \"Devices\" would land\n",
        "    on the wrong page. */}\n",
        "  </div>\n",
        ");\n",
    );
    let document = prepare_test_document(DiffSyntaxLanguage::Tsx, text);
    for ix in 2..=4 {
        let line = text.lines().nth(ix).expect("comment line");
        let tokens = syntax_tokens_for_prepared_document_line(document, ix)
            .map(|chunk| chunk.to_vec())
            .unwrap_or_default();
        let non_comment: Vec<_> = tokens
            .iter()
            .filter(|token| token.kind != SyntaxTokenKind::Comment)
            .map(|token| (&line[token.range.clone()], token.kind))
            .collect();
        // Only the braces holding the expression may be anything else.
        assert!(
            non_comment
                .iter()
                .all(|(text, _)| *text == "{" || *text == "}"),
            "line {ix} ({line:?}) coloured non-comment spans: {non_comment:?}"
        );
    }
}
