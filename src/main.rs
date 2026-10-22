mod image;
mod kitty;
mod math;
mod md;
mod render;
mod term;

use std::collections::HashMap;
use std::io::{BufWriter, ErrorKind, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, ExitStatus, Stdio};
use std::str::FromStr;

use clap::{CommandFactory, Parser};

const DEFAULT_MAX_WIDTH: usize = 100;
/// What `theme = "auto"` resolves to on a dark and on a light background.
const DARK_THEME: &str = "base16-ocean.dark";
const LIGHT_THEME: &str = "base16-ocean.light";
/// What `--theme` accepts: `auto` and syntect's bundled themes, which a test checks against syntect.
const THEMES: [&str; 8] = [
    "auto",
    DARK_THEME,
    LIGHT_THEME,
    "base16-eighties.dark",
    "base16-mocha.dark",
    "InspiredGitHub",
    "Solarized (dark)",
    "Solarized (light)",
];

const CONFIG_HELP: &str = "\
Config: $XDG_CONFIG_HOME/visomd/config.toml (default ~/.config/visomd/config.toml), one `key = value`
per line named like the long flags; `viso --print-config` lists them all. Flags override the file.";

/// Render Markdown in the terminal with LaTeX math typeset as inline kitty images.
#[derive(Parser)]
#[command(name = "viso", version, after_help = CONFIG_HELP)]
struct Args {
    /// Markdown file; "-" or omitted reads stdin
    file: Option<String>,
    /// Print math as raw LaTeX instead of images
    #[arg(long)]
    no_images: bool,
    /// Math ink color; default: the terminal's foreground
    #[arg(long, value_name = "RRGGBB")]
    fg: Option<String>,
    /// Wrap width in columns; default: the terminal width (80 when stdout is not a terminal), capped by --max-width
    #[arg(long, value_name = "N")]
    width: Option<usize>,
    /// Cap on the automatic wrap width, for readable lines on wide terminals [default: 100]
    #[arg(long, value_name = "N")]
    max_width: Option<usize>,
    /// Display math size relative to inline math and the prose; 1 = same size, as in LaTeX [default: 1]
    #[arg(long, value_name = "F")]
    display_scale: Option<f64>,
    /// syntect theme for code blocks; "auto" picks base16-ocean.dark or .light from the terminal
    /// background [default: auto]
    #[arg(long, value_name = "NAME", value_parser = THEMES)]
    theme: Option<String>,
    /// Page the text through $PAGER (default `less -R`); images go to the terminal directly
    #[arg(short, long)]
    pager: bool,
    /// Send image transfers to /dev/tty instead of stdout, as --pager does; your own pager must
    /// then stay on the main screen (less: `-RX` and LESSUTFCHARDEF=10EEEE:p)
    #[arg(long)]
    tty: bool,
    /// Print every config key with its effective value, as a config file, and exit
    #[arg(long)]
    print_config: bool,
    /// Print a shell completion script and exit
    #[arg(long, value_name = "SHELL", value_parser = ["bash", "zsh", "fish", "nushell"])]
    completions: Option<String>,
    /// Print what viso detects about the terminal and exit
    #[arg(long)]
    debug_term: bool,
}

/// Everything the config file can hold, after the flags are applied.
struct Settings {
    width: Option<usize>,
    max_width: usize,
    display_scale: f64,
    theme: String,
    fg: Option<String>,
    no_images: bool,
    pager: bool,
}

impl Settings {
    fn resolve(args: &Args, cfg: &HashMap<String, String>) -> Settings {
        Settings {
            width: args.width.or_else(|| setting(cfg, "width")),
            max_width: args
                .max_width
                .or_else(|| setting(cfg, "max-width"))
                .unwrap_or(DEFAULT_MAX_WIDTH),
            display_scale: args
                .display_scale
                .or_else(|| setting(cfg, "display-scale"))
                .unwrap_or(1.0),
            theme: args
                .theme
                .clone()
                .or_else(|| setting(cfg, "theme"))
                .unwrap_or_else(|| "auto".into()),
            fg: args.fg.clone().or_else(|| setting(cfg, "fg")),
            no_images: args.no_images || setting(cfg, "no-images").unwrap_or(false),
            pager: args.pager || setting(cfg, "pager").unwrap_or(false),
        }
    }

    /// Every key with its effective value, as a config file; unset keys as commented examples.
    fn to_toml(&self) -> String {
        let optional = |key: &str, value: Option<String>, example: &str, unset: &str| match value {
            Some(v) => format!("{key} = {v}\n"),
            None => format!("# {key} = {example}   # {unset}\n"),
        };
        let width = optional(
            "width",
            self.width.map(|w| w.to_string()),
            "120",
            "fixed wrap width; unset: the terminal width, capped by max-width",
        );
        let fg = optional(
            "fg",
            self.fg.as_ref().map(|f| format!("{f:?}")),
            "\"d3c6aa\"",
            "math ink color; unset: the terminal's foreground",
        );
        format!(
            "{width}max-width = {}   # cap on the automatic wrap width\n\
             display-scale = {:?}   # display math size relative to inline math\n\
             theme = {:?}   # syntect theme for code blocks; auto: by the terminal background\n\
             {fg}no-images = {}   # print math as LaTeX instead of images\n\
             pager = {}   # page the text through $PAGER\n",
            self.max_width, self.display_scale, self.theme, self.no_images, self.pager
        )
    }
}

fn config_path() -> Option<PathBuf> {
    let dir = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(d) => PathBuf::from(d),
        None => PathBuf::from(std::env::var_os("HOME")?).join(".config"),
    };
    Some(dir.join("visomd").join("config.toml"))
}

/// Flat `key = value` lines; `#` comments; optional double quotes around values.
// ponytail: hand-parsed TOML subset; switch to the toml crate if sections or arrays are ever needed.
fn load_config(path: Option<&PathBuf>) -> HashMap<String, String> {
    let Some(text) = path.and_then(|p| std::fs::read_to_string(p).ok()) else {
        return HashMap::new();
    };
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.starts_with('#') {
                return None;
            }
            let (k, v) = line.split_once('=')?;
            let v = v.trim();
            let v = match v.strip_prefix('"') {
                Some(quoted) => quoted.split('"').next().unwrap_or(""),
                None => v.split('#').next().unwrap_or("").trim(),
            };
            Some((k.trim().to_string(), v.to_string()))
        })
        .collect()
}

/// A typed config value; a malformed one is reported and ignored.
fn setting<T: FromStr>(cfg: &HashMap<String, String>, key: &str) -> Option<T> {
    let v = cfg.get(key)?;
    let parsed = v.parse().ok();
    if parsed.is_none() {
        eprintln!("viso: config: ignoring {key} = {v:?}");
    }
    parsed
}

fn parse_rgb(s: &str) -> Option<[u8; 3]> {
    let s = s.trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

/// The completion script for `shell`, one of the values `--completions` accepts.
fn completions(shell: &str) -> Vec<u8> {
    let mut cmd = Args::command();
    let mut out = Vec::new();
    match shell {
        "nushell" => {
            clap_complete::generate(clap_complete_nushell::Nushell, &mut cmd, "viso", &mut out)
        }
        other => {
            let shell: clap_complete::Shell = other.parse().expect("validated by clap");
            clap_complete::generate(shell, &mut cmd, "viso", &mut out);
        }
    }
    out
}

fn main() -> ExitCode {
    let args = Args::parse();
    if let Some(shell) = args.completions.as_deref() {
        let _ = std::io::Write::write_all(&mut std::io::stdout(), &completions(shell));
        return ExitCode::SUCCESS;
    }
    let config_path = config_path();
    let cfg = load_config(config_path.as_ref());
    let settings = Settings::resolve(&args, &cfg);
    let config_status = match &config_path {
        Some(p) if p.exists() => format!("{} ({} keys)", p.display(), cfg.len()),
        Some(p) => format!("{} (not found)", p.display()),
        None => "none ($HOME unset)".into(),
    };

    if args.print_config {
        print!(
            "# viso settings with flags applied; copy into {config_status}\n{}",
            settings.to_toml()
        );
        return ExitCode::SUCCESS;
    }
    let fg_override = match settings.fg.as_deref().map(parse_rgb) {
        Some(None) => {
            eprintln!("viso: --fg expects RRGGBB");
            return ExitCode::from(2);
        }
        Some(rgb) => rgb,
        None => None,
    };

    let mut tty = term::Tty::open();
    let stdout_tty = std::io::stdout().is_terminal();
    // like git, a pager only makes sense when the text would reach a terminal
    let pager = settings.pager && stdout_tty;
    if pager && tty.is_none() {
        eprintln!("viso: --pager needs a terminal (/dev/tty)");
        return ExitCode::from(2);
    }
    // whether the text is for this terminal: its width, hyperlinks and background apply
    let to_tty = stdout_tty || args.tty;
    let size = tty.as_ref().map(|t| t.size()).unwrap_or_default();
    let probe = !settings.no_images || args.debug_term;
    let graphics = probe && tty.as_ref().is_some_and(|t| t.supports_graphics());
    let fg_term = if probe && (fg_override.is_none() || args.debug_term) {
        tty.as_ref().and_then(|t| t.fg_color())
    } else {
        None
    };
    let auto_theme = settings.theme == "auto";
    let bg_term = if args.debug_term || (auto_theme && to_tty) {
        tty.as_ref().and_then(|t| t.bg_color())
    } else {
        None
    };
    let light = bg_term.is_some_and(is_light);
    let theme = match (auto_theme, light) {
        (false, _) => settings.theme.as_str(),
        (true, false) => DARK_THEME,
        (true, true) => LIGHT_THEME,
    };
    let hex = |c: Option<[u8; 3]>| {
        c.map_or("unknown".into(), |[r, g, b]| {
            format!("#{r:02x}{g:02x}{b:02x}")
        })
    };

    if args.debug_term {
        println!(
            "cols={} rows={} cell={}x{}px fg={} bg={} light={} graphics={} stdout_tty={}\nconfig={config_status}",
            size.cols,
            size.rows,
            size.cell_w,
            size.cell_h,
            hex(fg_term),
            hex(bg_term),
            if light { "yes" } else { "no" },
            graphics,
            stdout_tty
        );
        return ExitCode::SUCCESS;
    }

    let doc = match args.file.as_deref() {
        None | Some("-") => {
            let mut s = String::new();
            std::io::stdin().read_to_string(&mut s).map(|_| s)
        }
        Some(path) => std::fs::read_to_string(path),
    };
    let doc = match doc {
        Ok(d) => d,
        Err(e) => {
            eprintln!(
                "viso: cannot read {}: {e}",
                args.file.as_deref().unwrap_or("stdin")
            );
            return ExitCode::from(1);
        }
    };

    let auto_width = if to_tty && size.cols > 0 {
        usize::from(size.cols)
    } else {
        80
    };
    let width = settings
        .width
        .unwrap_or(auto_width.min(settings.max_width))
        .max(1);
    let images =
        (!settings.no_images && graphics && size.cell_w > 0 && size.cell_h > 0).then(|| {
            render::Images {
                cell_w: size.cell_w,
                cell_h: size.cell_h,
                fg: fg_override.or(fg_term).unwrap_or([128, 128, 128]),
                display_scale: settings.display_scale,
            }
        });

    let blocks = md::parse(&doc);
    // with a pager the text is rendered first and handed over whole, so every image is on the
    // terminal before the pager draws a placeholder
    let mut page_text = Vec::new();
    let mut stdout = BufWriter::new(std::io::stdout().lock());
    let out: &mut dyn Write = if pager { &mut page_text } else { &mut stdout };
    let mut renderer = render::Renderer::new(out, width, images, theme);
    renderer.hyperlinks = to_tty;
    renderer.light = light;
    renderer.base = match args.file.as_deref() {
        None | Some("-") => PathBuf::new(),
        Some(f) => Path::new(f).parent().unwrap_or(Path::new("")).to_path_buf(),
    };
    if args.tty || pager {
        renderer.transfers = tty.as_mut().map(|t| &mut t.0 as &mut dyn Write);
    }
    let rendered = renderer.render(&blocks);
    drop(renderer);
    if let Err(e) = rendered
        && e.kind() != ErrorKind::BrokenPipe
    {
        eprintln!("viso: {e}");
    }
    if !pager {
        return ExitCode::SUCCESS;
    }
    match page(&page_text) {
        Ok(status) => ExitCode::from(status.code().unwrap_or(1).clamp(0, 255) as u8),
        Err(e) => {
            eprintln!("viso: cannot run the pager: {e}");
            ExitCode::from(1)
        }
    }
}

/// `text` through `$PAGER` (default `less -R`, split on whitespace), returning its exit status.
/// less gets `-X`: kitty keeps images per screen buffer, so the placeholders only show on the main
/// screen, where the transfers went. `LESS` defaults to `R` so a plain `PAGER=less` shows the
/// colors too, and `LESSUTFCHARDEF` marks the placeholder character printable, since less
/// otherwise shows it as `<U+10EEEE>`.
fn page(text: &[u8]) -> std::io::Result<ExitStatus> {
    let pager = std::env::var("PAGER")
        .ok()
        .filter(|p| !p.trim().is_empty())
        .unwrap_or_else(|| "less -R".into());
    let mut words = pager.split_whitespace();
    let prog = words.next().unwrap_or("less");
    let mut cmd = Command::new(prog);
    cmd.args(words).stdin(Stdio::piped());
    if Path::new(prog).file_name().is_some_and(|n| n == "less") {
        let chardef = match std::env::var("LESSUTFCHARDEF") {
            Ok(v) if !v.is_empty() => format!("{v},10EEEE:p"),
            _ => "10EEEE:p".into(),
        };
        cmd.arg("-X")
            .env(
                "LESS",
                std::env::var_os("LESS").unwrap_or_else(|| "R".into()),
            )
            .env("LESSUTFCHARDEF", chardef);
    }
    let mut child = cmd.spawn()?;
    let mut stdin = child.stdin.take().expect("piped stdin");
    if let Err(e) = stdin.write_all(text)
        && e.kind() != ErrorKind::BrokenPipe
    {
        return Err(e);
    }
    drop(stdin);
    child.wait()
}

/// Relative luminance of an sRGB color above one half: light enough for a light theme.
fn is_light([r, g, b]: [u8; 3]) -> bool {
    let lum = 0.2126 * f64::from(r) + 0.7152 * f64::from(g) + 0.0722 * f64::from(b);
    lum / 255.0 > 0.5
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_lines_parse() {
        let dir = std::env::temp_dir().join(format!("visomd-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            "# comment\nmax-width = 90 # trailing\ntheme = \"gruvbox # not a comment\"\nfg = \"#d3c6aa\"\nno-images = true\nbad =\n",
        )
        .unwrap();
        let cfg = load_config(Some(&path));
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(setting::<usize>(&cfg, "max-width"), Some(90));
        assert_eq!(
            setting::<String>(&cfg, "theme").as_deref(),
            Some("gruvbox # not a comment")
        );
        assert_eq!(cfg["fg"], "#d3c6aa");
        assert_eq!(setting::<bool>(&cfg, "no-images"), Some(true));
        assert_eq!(setting::<usize>(&cfg, "bad"), None);
        assert_eq!(setting::<usize>(&cfg, "missing"), None);
        assert!(load_config(Some(&dir.join("nope"))).is_empty());
    }

    #[test]
    fn completions_cover_the_flags() {
        for shell in ["bash", "zsh", "fish", "nushell"] {
            let script = String::from_utf8(completions(shell)).unwrap();
            assert!(script.contains("display-scale"), "{shell}: {script}");
            assert!(
                script.contains("InspiredGitHub"),
                "{shell}: theme values: {script}"
            );
        }
        assert!(
            String::from_utf8(completions("nushell"))
                .unwrap()
                .contains("export extern viso")
        );
    }

    #[test]
    fn printed_config_round_trips_through_the_parser() {
        let s = Settings {
            width: Some(90),
            max_width: 100,
            display_scale: 1.5,
            theme: "gruvbox".into(),
            fg: None,
            no_images: true,
            pager: true,
        };
        let dir = std::env::temp_dir().join(format!("visomd-print-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, s.to_toml()).unwrap();
        let cfg = load_config(Some(&path));
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(setting::<usize>(&cfg, "width"), Some(90));
        assert_eq!(setting::<f64>(&cfg, "display-scale"), Some(1.5));
        assert_eq!(setting::<String>(&cfg, "theme").as_deref(), Some("gruvbox"));
        assert_eq!(setting::<bool>(&cfg, "no-images"), Some(true));
        assert_eq!(setting::<bool>(&cfg, "pager"), Some(true));
        assert!(!cfg.contains_key("fg"), "unset keys are commented out");
    }

    #[test]
    fn theme_names_are_syntects_bundled_ones() {
        let mut listed: Vec<&str> = THEMES[1..].to_vec();
        listed.sort_unstable();
        let mut bundled: Vec<String> = syntect::highlighting::ThemeSet::load_defaults()
            .themes
            .into_keys()
            .collect();
        bundled.sort_unstable();
        assert_eq!(listed, bundled);
    }

    #[test]
    fn luminance_splits_light_from_dark() {
        assert!(is_light([255, 255, 255]));
        assert!(is_light([0xef, 0xf1, 0xf5])); // base16-ocean.light background
        assert!(!is_light([0, 0, 0]));
        assert!(!is_light([0x2b, 0x30, 0x3b])); // base16-ocean.dark background
        assert!(!is_light([0, 0, 255]), "pure blue is dark");
        assert!(is_light([0, 255, 0]), "pure green is light");
    }
}
