use std::collections::HashMap;
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
) -> HashMap<&'static str, &'static str> {
    let mut map = HashMap::new();
    for (icon_key, associations) in associations_by_icon_key {
        for association in *associations {
            map.insert(*association, *icon_key);
        }
    }
    map
}

static FILE_STEMS: LazyLock<HashMap<&'static str, &'static str>> =
    LazyLock::new(|| icon_keys_by_association(FILE_STEMS_BY_ICON_KEY));

static FILE_SUFFIXES: LazyLock<HashMap<&'static str, &'static str>> =
    LazyLock::new(|| icon_keys_by_association(FILE_SUFFIXES_BY_ICON_KEY));

static FILE_ICON_PATHS: LazyLock<HashMap<&'static str, &'static str>> =
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

    // Multi-part suffix (e.g. `Component.stories.tsx` -> `stories.tsx`).
    if let Some(suffix) = multiple_extensions(path)
        && let Some(icon) = icon_for_suffix(&suffix)
    {
        return icon;
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

/// Port of Zed's `PathExt::multiple_extensions`.
fn multiple_extensions(path: &Path) -> Option<String> {
    let file_name = path.file_name()?.to_str()?;
    // Skip the file stem; keep only the dotted suffixes.
    let parts: Vec<&str> = file_name.split('.').skip(1).collect();
    if parts.len() < 2 {
        return None;
    }
    Some(parts.join("."))
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

    #[test]
    fn resolves_full_names_and_hidden_files() {
        assert_eq!(icon("Dockerfile"), "icons/file_icons/docker.svg");
        assert_eq!(icon(".gitignore"), "icons/file_icons/git.svg");
        assert_eq!(icon(".editorconfig"), "icons/file_icons/editorconfig.svg");
        assert_eq!(
            icon("eslint.config.js"),
            "icons/file_icons/eslint.svg"
        );
        assert_eq!(
            icon(".gitlab-ci.yml"),
            "icons/file_icons/gitlab.svg"
        );
    }

    #[test]
    fn unknown_extension_falls_back_to_default() {
        assert_eq!(icon("mystery.zzz"), "icons/file_icons/file.svg");
        assert_eq!(icon("README"), "icons/file_icons/file.svg");
    }
}
