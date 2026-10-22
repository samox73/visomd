#!/usr/bin/env nu
# Screenshot `viso DOC` in a kitty on a throwaway headless sway: nothing opens on your desktop.
# Needs sway, grim, kitty, imagemagick and nu in PATH, e.g.
#
#   nix shell nixpkgs#sway nixpkgs#grim nixpkgs#imagemagick -c nu assets/screenshot.nu                 # assets/demo.md -> assets/demo.png
#   nix shell nixpkgs#sway nixpkgs#grim nixpkgs#imagemagick -c nu assets/screenshot.nu notes.md x.png --cols 90 --rows 40

const title = "viso-screenshot"
const assets = path self .

# The window called `title` in sway's tree, with its logical rectangle.
def find-window [node: record]: nothing -> any {
    if ($node.name? == $title) {
        return $node
    }
    for child in (($node.nodes? | default []) ++ ($node.floating_nodes? | default [])) {
        let found = find-window $child
        if $found != null {
            return $found
        }
    }
    null
}

def wait-until [what: string, check: closure, --timeout: duration = 20sec] {
    let deadline = (date now) + $timeout
    while not (do $check) {
        if (date now) > $deadline {
            error make {msg: $"($what) did not come up"}
        }
        sleep 100ms
    }
}

def main [doc: string = "assets/demo.md", out: string = "assets/demo.png", --cols: int = 100, --rows: int = 45] {
    ^cargo build --release --quiet
    let viso = $env.PWD | path join target release viso
    let doc = $doc | path expand
    let out = $out | path expand
    # sway's socket path must fit in 108 bytes, so keep the runtime dir short
    let run = "/tmp" | path join $"viso-shot-(random chars --length 6)"
    mkdir $run
    ^chmod 700 $run
    let session = {XDG_RUNTIME_DIR: $run, WAYLAND_DISPLAY: wayland-1}

    let sway = job spawn {
        with-env {XDG_RUNTIME_DIR: $run, WLR_BACKENDS: headless, WLR_RENDERER: pixman, WLR_LIBINPUT_NO_DEVICES: "1"} {
            hide-env -i WAYLAND_DISPLAY DISPLAY SWAYSOCK
            ^sway -c ($assets | path join sway.conf) out+err> ($run | path join sway.log)
        }
    }
    try {
        wait-until "sway" { $run | path join wayland-1 | path exists }
        let sock = glob ($run | path join "sway-ipc.*.sock") | first
        let kitty = job spawn {
            with-env ($session | merge {LIBGL_ALWAYS_SOFTWARE: "1", LP_NUM_THREADS: "4"}) {
                hide-env -i DISPLAY
                ^kitty --title $title -o remember_window_size=no -o initial_window_width=$"($cols + 2)c" -o initial_window_height=$"($rows)c" -o window_padding_width=16 -o font_size=12 sh -c $"stty -echo; tput civis; clear; sleep 1; ($viso) --width ($cols) ($doc); sleep 30" out+err> ($run | path join kitty.log)
            }
        }
        let window = {|| find-window (^swaymsg -s $sock -t get_tree | from json) }
        wait-until "the kitty window" { (do $window) != null }
        sleep 2500ms # let kitty settle, viso render and the images draw
        let r = (do $window).rect
        with-env $session { ^grim -g $"($r.x),($r.y) ($r.width)x($r.height)" $out }
        job kill $kitty
        # crop to the rendered content and pad it evenly, so the row count never has to match the document
        let bg = ^magick $out -format "%[pixel:p{0,0}]" info:
        ^magick $out -trim +repage -bordercolor $bg -border 48 $out
        let size = ^magick $out -format "%wx%h" info:
        print $"wrote ($out) \(($size) px\)"
    } catch {|e|
        print $"failed: ($e.msg); logs in ($run)"
        job kill $sway
        return
    }
    job kill $sway
    rm -rf $run
}
