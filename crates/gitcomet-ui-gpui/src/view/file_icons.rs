use rustc_hash::FxHashMap;
use std::path::Path;
use std::sync::LazyLock;

// File-type icon resolution ported from Zed's default icon theme so the file
// browser matches Zed. The three tables below are transcribed verbatim from
// `zed/crates/theme/src/icon_theme.rs` and the resolution order in
// `file_icon_for_path` mirrors `zed/crates/file_icons/src/file_icons.rs`
// (`FileIcons::get_icon`).

/// Full file names mapped to an icon key.
const FILE_STEMS_BY_ICON_KEY: &[(&str, &[&str])] = &[
    ("docker", &["Containerfile", "Dockerfile"]),
    ("ruby", &["Podfile"]),
    ("heroku", &["Procfile"]),
];

/// File suffixes (extensions, multi-part suffixes, and a few full names) mapped
/// to an icon key.
const FILE_SUFFIXES_BY_ICON_KEY: &[(&str, &[&str])] = &[
    // Not in Zed's icon theme, glyph and mapping both: every assembly dialect
    // GitComet highlights fell back to the blank page glyph, which in a directory
    // of `.s` files reads as "none of these are source". `assembly.svg` is a chip
    // outline drawn to this set's conventions -- 16x16, 1.2 stroke, round caps --
    // rather than the generic code braces, which said only "some language".
    //
    // `S` is listed beside `s` because this lookup is case-sensitive and
    // preprocessed assembly is conventionally capitalised -- `head.S`, `crt0.S`
    // -- which is most of the `.s` files in a kernel or libc tree. The other
    // dialects have no capitalised convention worth carrying.
    //
    // `il`, `ll`, `spvasm`, `wat` and `wast` are the assembly-adjacent
    // intermediate forms -- CIL, LLVM IR, SPIR-V, WebAssembly text. They share
    // the chip because they are the same kind of thing to a reader scanning a
    // directory: an instruction stream rather than a program.
    (
        "assembly",
        &["S", "asm", "il", "ll", "nasm", "s", "spvasm", "wast", "wat"],
    ),
    ("astro", &["astro"]),
    (
        "audio",
        &[
            "aac", "flac", "m4a", "mka", "mp3", "ogg", "opus", "wav", "wma", "wv",
        ],
    ),
    ("backup", &["bak"]),
    ("ballerina", &["bal"]),
    ("bicep", &["bicep"]),
    ("bun", &["lockb"]),
    ("c", &["c", "h"]),
    ("cairo", &["cairo"]),
    ("code", &["handlebars", "metadata", "rkt", "scm"]),
    ("coffeescript", &["coffee"]),
    (
        "cpp",
        &[
            "c++", "h++", "cc", "cpp", "cppm", "cxx", "hh", "hpp", "hxx", "inl", "ixx",
        ],
    ),
    ("crystal", &["cr", "ecr"]),
    ("csharp", &["cs"]),
    ("csproj", &["csproj"]),
    ("css", &["css", "pcss", "postcss"]),
    ("cue", &["cue"]),
    ("dart", &["dart"]),
    ("diff", &["diff"]),
    (
        "docker",
        &[
            "docker-compose.yml",
            "docker-compose.yaml",
            "compose.yml",
            "compose.yaml",
        ],
    ),
    (
        "document",
        &[
            "doc", "docx", "mdx", "odp", "ods", "odt", "pdf", "ppt", "pptx", "rtf", "txt", "xls",
            "xlsx",
        ],
    ),
    ("editorconfig", &["editorconfig"]),
    ("elixir", &["eex", "ex", "exs", "heex", "leex", "neex"]),
    ("elm", &["elm"]),
    (
        "erlang",
        &[
            "Emakefile",
            "app.src",
            "erl",
            "escript",
            "hrl",
            "rebar.config",
            "xrl",
            "yrl",
        ],
    ),
    (
        "eslint",
        &[
            "eslint.config.cjs",
            "eslint.config.cts",
            "eslint.config.js",
            "eslint.config.mjs",
            "eslint.config.mts",
            "eslint.config.ts",
            "eslintrc",
            "eslintrc.js",
            "eslintrc.json",
        ],
    ),
    ("font", &["otf", "ttf", "woff", "woff2"]),
    ("fsharp", &["fs"]),
    ("fsproj", &["fsproj"]),
    ("gitlab", &["gitlab-ci.yml", "gitlab-ci.yaml"]),
    ("gleam", &["gleam"]),
    ("go", &["go", "mod", "work"]),
    ("graphql", &["gql", "graphql", "graphqls"]),
    ("haskell", &["hs"]),
    ("hcl", &["hcl"]),
    (
        "helm",
        &[
            "helmfile.yaml",
            "helmfile.yml",
            "Chart.yaml",
            "Chart.yml",
            "Chart.lock",
            "values.yaml",
            "values.yml",
            "requirements.yaml",
            "requirements.yml",
            "tpl",
        ],
    ),
    ("html", &["htm", "html"]),
    (
        "image",
        &[
            "avif", "bmp", "gif", "heic", "heif", "ico", "j2k", "jfif", "jp2", "jpeg", "jpg",
            "jxl", "png", "psd", "qoi", "svg", "tiff", "webp",
        ],
    ),
    ("ipynb", &["ipynb"]),
    ("java", &["java"]),
    ("javascript", &["cjs", "js", "mjs"]),
    // Not in Zed's icon theme: GitComet highlights Nunjucks/Jinja templates (see
    // DiffSyntaxLanguage::Jinja) and a template file falling back to the generic
    // page icon reads as "unknown type". The split between the two keys is
    // cosmetic -- one grammar serves both -- but `.njk` and `.j2` come from
    // different ecosystems and users sort by icon.
    ("jinja", &["j2", "jinja", "jinja2", "twig", "dj"]),
    ("json", &["json", "jsonc"]),
    ("julia", &["jl"]),
    ("kdl", &["kdl"]),
    ("kotlin", &["kt"]),
    ("lock", &["lock"]),
    ("log", &["log"]),
    ("lua", &["lua"]),
    ("luau", &["luau"]),
    ("markdown", &["markdown", "md"]),
    ("metal", &["metal"]),
    ("nim", &["nim", "nims", "nimble"]),
    ("nix", &["nix"]),
    ("nunjucks", &["njk", "nunjucks"]), // see the `jinja` entry above
    ("ocaml", &["ml", "mli", "mlx"]),
    ("odin", &["odin"]),
    ("php", &["php"]),
    (
        "prettier",
        &[
            "prettier.config.cjs",
            "prettier.config.js",
            "prettier.config.mjs",
            "prettierignore",
            "prettierrc",
            "prettierrc.cjs",
            "prettierrc.js",
            "prettierrc.json",
            "prettierrc.json5",
            "prettierrc.mjs",
            "prettierrc.toml",
            "prettierrc.yaml",
            "prettierrc.yml",
        ],
    ),
    ("prisma", &["prisma"]),
    ("puppet", &["pp"]),
    ("python", &["py"]),
    ("r", &["r", "R"]),
    ("react", &["cjsx", "ctsx", "jsx", "mjsx", "mtsx", "tsx"]),
    ("roc", &["roc"]),
    ("ruby", &["rb"]),
    ("rust", &["rs"]),
    ("sass", &["sass", "scss"]),
    ("scala", &["scala", "sc"]),
    ("settings", &["conf", "ini"]),
    ("solidity", &["sol"]),
    (
        "storage",
        &[
            "accdb", "csv", "dat", "db", "dbf", "dll", "fmp", "fp7", "frm", "gdb", "ib", "ldf",
            "mdb", "mdf", "myd", "myi", "pdb", "RData", "rdata", "sav", "sdf", "sql", "sqlite",
            "tsv",
        ],
    ),
    (
        "stylelint",
        &[
            "stylelint.config.cjs",
            "stylelint.config.js",
            "stylelint.config.mjs",
            "stylelintignore",
            "stylelintrc",
            "stylelintrc.cjs",
            "stylelintrc.js",
            "stylelintrc.json",
            "stylelintrc.mjs",
            "stylelintrc.yaml",
            "stylelintrc.yml",
        ],
    ),
    ("surrealql", &["surql"]),
    ("svelte", &["svelte"]),
    ("swift", &["swift"]),
    ("tcl", &["tcl"]),
    ("template", &["hbs", "plist", "xml"]),
    (
        "terminal",
        &[
            "bash",
            "bash_aliases",
            "bash_login",
            "bash_logout",
            "bash_profile",
            "bashrc",
            "fish",
            "nu",
            "profile",
            "ps1",
            "sh",
            "zlogin",
            "zlogout",
            "zprofile",
            "zsh",
            "zsh_aliases",
            "zsh_histfile",
            "zsh_history",
            "zshenv",
            "zshrc",
        ],
    ),
    ("terraform", &["tf", "tfvars"]),
    ("toml", &["toml"]),
    ("typescript", &["cts", "mts", "ts"]),
    ("v", &["v", "vsh", "vv"]),
    (
        "vcs",
        &[
            "COMMIT_EDITMSG",
            "EDIT_DESCRIPTION",
            "MERGE_MSG",
            "NOTES_EDITMSG",
            "TAG_EDITMSG",
            "gitattributes",
            "gitignore",
            "gitkeep",
            "gitmodules",
        ],
    ),
    ("vbproj", &["vbproj"]),
    ("video", &["avi", "m4v", "mkv", "mov", "mp4", "webm", "wmv"]),
    ("vs_sln", &["sln"]),
    ("vs_suo", &["suo"]),
    ("vue", &["vue"]),
    ("vyper", &["vy", "vyi"]),
    ("wgsl", &["wgsl"]),
    ("yaml", &["yaml", "yml"]),
    ("zig", &["zig"]),
];

/// Icon keys mapped to their SVG asset path. Keys with no dedicated glyph point
/// at `file.svg`, exactly as in Zed's default theme.
const FILE_ICONS: &[(&str, &str)] = &[
    ("assembly", "icons/file_icons/assembly.svg"),
    ("astro", "icons/file_icons/astro.svg"),
    ("audio", "icons/file_icons/audio.svg"),
    ("ballerina", "icons/file_icons/ballerina.svg"),
    ("bicep", "icons/file_icons/file.svg"),
    ("bun", "icons/file_icons/bun.svg"),
    ("c", "icons/file_icons/c.svg"),
    ("cairo", "icons/file_icons/cairo.svg"),
    ("code", "icons/file_icons/code.svg"),
    ("coffeescript", "icons/file_icons/coffeescript.svg"),
    ("cpp", "icons/file_icons/cpp.svg"),
    ("crystal", "icons/file_icons/file.svg"),
    ("csharp", "icons/file_icons/file.svg"),
    ("csproj", "icons/file_icons/file.svg"),
    ("css", "icons/file_icons/css.svg"),
    ("cue", "icons/file_icons/file.svg"),
    ("dart", "icons/file_icons/dart.svg"),
    ("default", "icons/file_icons/file.svg"),
    ("diff", "icons/file_icons/diff.svg"),
    ("docker", "icons/file_icons/docker.svg"),
    ("document", "icons/file_icons/book.svg"),
    ("editorconfig", "icons/file_icons/editorconfig.svg"),
    ("elixir", "icons/file_icons/elixir.svg"),
    ("elm", "icons/file_icons/elm.svg"),
    ("erlang", "icons/file_icons/erlang.svg"),
    ("eslint", "icons/file_icons/eslint.svg"),
    ("font", "icons/file_icons/font.svg"),
    ("fsharp", "icons/file_icons/fsharp.svg"),
    ("fsproj", "icons/file_icons/file.svg"),
    ("gitlab", "icons/file_icons/gitlab.svg"),
    ("gleam", "icons/file_icons/gleam.svg"),
    ("go", "icons/file_icons/go.svg"),
    ("graphql", "icons/file_icons/graphql.svg"),
    ("haskell", "icons/file_icons/haskell.svg"),
    ("hcl", "icons/file_icons/hcl.svg"),
    ("helm", "icons/file_icons/helm.svg"),
    ("heroku", "icons/file_icons/heroku.svg"),
    ("html", "icons/file_icons/html.svg"),
    ("image", "icons/file_icons/image.svg"),
    ("ipynb", "icons/file_icons/jupyter.svg"),
    ("java", "icons/file_icons/java.svg"),
    ("javascript", "icons/file_icons/javascript.svg"),
    ("jinja", "icons/file_icons/jinja.svg"),
    ("json", "icons/file_icons/code.svg"),
    ("julia", "icons/file_icons/julia.svg"),
    ("kdl", "icons/file_icons/kdl.svg"),
    ("kotlin", "icons/file_icons/kotlin.svg"),
    ("lock", "icons/file_icons/lock.svg"),
    ("log", "icons/file_icons/info.svg"),
    ("lua", "icons/file_icons/lua.svg"),
    ("luau", "icons/file_icons/luau.svg"),
    ("markdown", "icons/file_icons/book.svg"),
    ("metal", "icons/file_icons/metal.svg"),
    ("nim", "icons/file_icons/nim.svg"),
    ("nix", "icons/file_icons/nix.svg"),
    ("nunjucks", "icons/file_icons/nunjucks.svg"),
    ("ocaml", "icons/file_icons/ocaml.svg"),
    ("odin", "icons/file_icons/odin.svg"),
    ("phoenix", "icons/file_icons/phoenix.svg"),
    ("php", "icons/file_icons/php.svg"),
    ("prettier", "icons/file_icons/prettier.svg"),
    ("prisma", "icons/file_icons/prisma.svg"),
    ("puppet", "icons/file_icons/puppet.svg"),
    ("python", "icons/file_icons/python.svg"),
    ("r", "icons/file_icons/r.svg"),
    ("react", "icons/file_icons/react.svg"),
    ("roc", "icons/file_icons/roc.svg"),
    ("ruby", "icons/file_icons/ruby.svg"),
    ("rust", "icons/file_icons/rust.svg"),
    ("sass", "icons/file_icons/sass.svg"),
    ("scala", "icons/file_icons/scala.svg"),
    ("settings", "icons/file_icons/settings.svg"),
    ("solidity", "icons/file_icons/file.svg"),
    ("storage", "icons/file_icons/database.svg"),
    ("stylelint", "icons/file_icons/javascript.svg"),
    ("surrealql", "icons/file_icons/surrealql.svg"),
    ("svelte", "icons/file_icons/html.svg"),
    ("swift", "icons/file_icons/swift.svg"),
    ("tcl", "icons/file_icons/tcl.svg"),
    ("template", "icons/file_icons/html.svg"),
    ("terminal", "icons/file_icons/terminal.svg"),
    ("terraform", "icons/file_icons/terraform.svg"),
    ("toml", "icons/file_icons/toml.svg"),
    ("typescript", "icons/file_icons/typescript.svg"),
    ("v", "icons/file_icons/v.svg"),
    ("vbproj", "icons/file_icons/file.svg"),
    ("vcs", "icons/file_icons/git.svg"),
    ("video", "icons/file_icons/video.svg"),
    ("vs_sln", "icons/file_icons/file.svg"),
    ("vs_suo", "icons/file_icons/file.svg"),
    ("vue", "icons/file_icons/vue.svg"),
    ("vyper", "icons/file_icons/vyper.svg"),
    ("wgsl", "icons/file_icons/wgsl.svg"),
    ("yaml", "icons/file_icons/yaml.svg"),
    ("zig", "icons/file_icons/zig.svg"),
];

fn icon_keys_by_association(
    associations_by_icon_key: &[(&'static str, &'static [&'static str])],
) -> FxHashMap<&'static str, &'static str> {
    let capacity = associations_by_icon_key
        .iter()
        .map(|(_, associations)| associations.len())
        .sum();
    let mut map = FxHashMap::with_capacity_and_hasher(capacity, Default::default());
    for (icon_key, associations) in associations_by_icon_key {
        for association in *associations {
            map.insert(*association, *icon_key);
        }
    }
    map
}

static FILE_STEMS: LazyLock<FxHashMap<&'static str, &'static str>> =
    LazyLock::new(|| icon_keys_by_association(FILE_STEMS_BY_ICON_KEY));

static FILE_SUFFIXES: LazyLock<FxHashMap<&'static str, &'static str>> =
    LazyLock::new(|| icon_keys_by_association(FILE_SUFFIXES_BY_ICON_KEY));

static FILE_ICON_PATHS: LazyLock<FxHashMap<&'static str, &'static str>> =
    LazyLock::new(|| FILE_ICONS.iter().copied().collect());

fn default_file_icon() -> &'static str {
    FILE_ICON_PATHS
        .get("default")
        .copied()
        .unwrap_or("icons/file_icons/file.svg")
}

/// Look up an icon for a candidate suffix/name, consulting the stem map first
/// then the suffix map, then resolving the icon key to a path. Returns `None`
/// when the key has no dedicated glyph so the caller can keep trying.
fn icon_for_suffix(suffix: &str) -> Option<&'static str> {
    let typ = FILE_STEMS
        .get(suffix)
        .or_else(|| FILE_SUFFIXES.get(suffix))?;
    FILE_ICON_PATHS.get(*typ).copied()
}

/// Resolve a file's icon, mirroring Zed's `FileIcons::get_icon` strategy order.
pub fn file_icon_for_path(path: &Path) -> &'static str {
    if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
        // Full file name (e.g. `eslint.config.js`, `docker-compose.yml`).
        if let Some(icon) = icon_for_suffix(file_name) {
            return icon;
        }
        // Progressively drop the leading dotted segment (e.g. `auth.module.js`
        // -> `module.js` -> `js`).
        let mut typ = file_name;
        while let Some((_, suffix)) = typ.split_once('.') {
            if let Some(icon) = icon_for_suffix(suffix) {
                return icon;
            }
            typ = suffix;
        }
    }

    // Extension or hidden-file name (e.g. `.gitignore` -> `gitignore`,
    // `data.json` -> `json`).
    if let Some(suffix) = extension_or_hidden_file_name(path)
        && let Some(icon) = icon_for_suffix(suffix)
    {
        return icon;
    }

    // Plain extension fallback for the remaining hidden-file cases
    // (e.g. `.data.json` -> `json`).
    if let Some(ext) = path.extension().and_then(|e| e.to_str())
        && let Some(icon) = icon_for_suffix(ext)
    {
        return icon;
    }

    default_file_icon()
}

pub fn folder_icon(expanded: bool) -> &'static str {
    if expanded {
        "icons/file_icons/folder_open.svg"
    } else {
        "icons/file_icons/folder.svg"
    }
}

pub fn chevron_icon(expanded: bool) -> &'static str {
    if expanded {
        "icons/chevron_down.svg"
    } else {
        "icons/arrow_right.svg"
    }
}

/// Characteristic tint for a resolved file-icon path, keyed on well-known
/// technology brand colors so file types read at a glance in the tree.
/// Types without a strong identity return `None` and keep the caller's
/// neutral tint. Each entry carries a (dark-theme, light-theme) pair: bright
/// pastels for dark surfaces, deeper shades for light ones.
pub fn file_icon_color(icon_path: &str, is_dark: bool) -> Option<gpui::Rgba> {
    let key = icon_path
        .strip_prefix("icons/file_icons/")?
        .strip_suffix(".svg")?;
    let (dark, light): (u32, u32) = match key {
        "rust" => (0xF2A374, 0xCE422B),
        "javascript" => (0xF0DB4F, 0xA38F00),
        "typescript" => (0x6CB6F5, 0x3178C6),
        "react" => (0x61DAFB, 0x0E7490),
        "python" | "jupyter" | "notebook" => (0x6CA9E8, 0x3776AB),
        "go" => (0x53C6E8, 0x00758F),
        "c" => (0x7CA8DC, 0x03599C),
        "cpp" => (0x6AA1D8, 0x00599C),
        "java" => (0xF0A05A, 0xC25E00),
        "ruby" => (0xF08A84, 0xB32821),
        "php" => (0xA2A6DC, 0x6A6EA8),
        "phoenix" => (0xF09B6C, 0xC75A22),
        "html" => (0xF0824F, 0xD04A22),
        "css" => (0x6BB4F0, 0x1572B6),
        "sass" => (0xE689B8, 0xBF5590),
        "vue" => (0x6BD3A0, 0x2F9668),
        "nunjucks" => (0x7FBF6E, 0x3D8137),
        "jinja" => (0xD86A6A, 0xB41717),
        "astro" => (0xF09668, 0xC6551A),
        "yaml" => (0xB8A2D8, 0x7A5C9E),
        "toml" => (0xC79378, 0x9C5B3C),
        "git" => (0xF0825E, 0xD14425),
        "gitlab" => (0xFC8E55, 0xD9591C),
        "docker" => (0x5FB2F5, 0x1D7DC4),
        "database" | "surrealql" => (0x7EB8D8, 0x3A7A9C),
        "elixir" => (0xB491C9, 0x6E4A7E),
        "erlang" => (0xE06D8A, 0xA90533),
        "haskell" => (0x9E8FD0, 0x5E5086),
        "kotlin" => (0xA98BFF, 0x6B45D6),
        "swift" => (0xF58469, 0xD6432C),
        "dart" => (0x55B0EE, 0x0175C2),
        "scala" => (0xED7A72, 0xC42B24),
        "lua" | "luau" => (0x8298E8, 0x3A50B4),
        "zig" => (0xF7B953, 0xC77F0A),
        "nix" => (0x82A4E8, 0x4468AE),
        "gleam" => (0xFFAFF3, 0xC875BC),
        "julia" => (0xBC8AD8, 0x8450A0),
        "r" => (0x6FA3E8, 0x276DC3),
        "ocaml" => (0xF59A55, 0xC9560D),
        "elm" => (0x7FD0E5, 0x3E93AA),
        "terraform" => (0xA97FE0, 0x6B38A6),
        "graphql" => (0xF063C0, 0xC00080),
        "eslint" => (0x8973E0, 0x4B32C3),
        "fsharp" => (0x6FB4DC, 0x2F7AA4),
        "coffeescript" => (0xA87E60, 0x6F4E37),
        "heroku" => (0x9B72D8, 0x5A2CA0),
        "image" | "camera" => (0x8FCE8A, 0x3F8C3A),
        "audio" => (0xCE9AE0, 0x8E4AA8),
        "video" => (0xE09AB8, 0xB04A78),
        "lock" => (0xD8C078, 0xA08830),
        "archive" => (0xD0B080, 0x8C6E3C),
        _ => return None,
    };
    let rgb = if is_dark { dark } else { light };
    Some(gpui::rgba((rgb << 8) | 0xFF))
}

/// Port of Zed's `PathExt::extension_or_hidden_file_name`.
fn extension_or_hidden_file_name(path: &Path) -> Option<&str> {
    let file_name = path.file_name()?.to_str()?;
    if file_name.starts_with('.') {
        return file_name.strip_prefix('.');
    }
    path.extension()
        .and_then(|e| e.to_str())
        .or_else(|| path.file_stem()?.to_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn icon(name: &str) -> &'static str {
        file_icon_for_path(Path::new(name))
    }

    #[test]
    fn resolves_common_languages() {
        assert_eq!(icon("src/main.rs"), "icons/file_icons/rust.svg");
        assert_eq!(icon("app.ts"), "icons/file_icons/typescript.svg");
        assert_eq!(icon("App.tsx"), "icons/file_icons/react.svg");
        assert_eq!(icon("main.go"), "icons/file_icons/go.svg");
        assert_eq!(icon("script.py"), "icons/file_icons/python.svg");
    }

    #[test]
    fn resolves_zed_specific_mappings() {
        // json uses the generic code glyph in Zed's theme.
        assert_eq!(icon("package.json"), "icons/file_icons/code.svg");
        // svelte/xml fall back to the html glyph.
        assert_eq!(icon("Page.svelte"), "icons/file_icons/html.svg");
        assert_eq!(icon("data.xml"), "icons/file_icons/html.svg");
        // csv/sql share the storage (database) glyph.
        assert_eq!(icon("rows.csv"), "icons/file_icons/database.svg");
        // C# has no dedicated glyph in Zed's default theme.
        assert_eq!(icon("Program.cs"), "icons/file_icons/file.svg");
    }

    /// Every assembly dialect GitComet highlights has an icon.
    ///
    /// They all share the generic code glyph -- the point of the test is that
    /// none of them falls through to `file.svg`, which is what a reader sees as
    /// "unknown type".
    #[test]
    fn resolves_assembly_dialects() {
        for name in [
            "boot/head.S",
            "src/main.s",
            "hello.asm",
            "hello.nasm",
            "bin/hello.il",
            "build/hello.ll",
            "shaders/frag.spvasm",
            "build/module.wat",
            "build/module.wast",
        ] {
            assert_eq!(
                icon(name),
                "icons/file_icons/assembly.svg",
                "{name} should not fall back to the blank page glyph"
            );
        }
    }

    #[test]
    fn resolves_full_names_and_hidden_files() {
        assert_eq!(icon("Dockerfile"), "icons/file_icons/docker.svg");
        assert_eq!(icon(".gitignore"), "icons/file_icons/git.svg");
        assert_eq!(icon(".editorconfig"), "icons/file_icons/editorconfig.svg");
        assert_eq!(icon("eslint.config.js"), "icons/file_icons/eslint.svg");
        assert_eq!(icon(".gitlab-ci.yml"), "icons/file_icons/gitlab.svg");
        assert_eq!(
            icon("project.eslint.config.js"),
            "icons/file_icons/eslint.svg"
        );
        assert_eq!(icon(".eslint.config.js"), "icons/file_icons/eslint.svg");
    }

    #[test]
    fn unknown_extension_falls_back_to_default() {
        assert_eq!(icon("mystery.zzz"), "icons/file_icons/file.svg");
        assert_eq!(icon("README"), "icons/file_icons/file.svg");
    }
}
