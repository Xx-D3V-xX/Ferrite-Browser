## What is in this release

A fix release. Everything below landed since build 52 (`91441a0`). Each item has a row in
`docs/TO-DO.md` (the `T-` numbers) saying exactly how it was checked.

### Downloads that start

- **Windows: no more "VCRUNTIME140.dll was not found".** The Windows packages now carry
  the Visual C++ runtime next to `ferrite.exe` (`MSVCP140.dll`, `VCRUNTIME140.dll`,
  `VCRUNTIME140_1.dll`...). Every CI machine has it installed, so earlier builds started
  there and nowhere else. Each package is now checked before it is published: every DLL
  any file in it loads must be in the package or part of Windows (T-336).
- **Linux media build: no more "libgstplay-1.0.so.0: cannot open shared object file".**
  The `-media` package now carries GStreamer and everything it needs in `lib/` beside
  `ferrite`; you install nothing. Keep the folder together (`./install.sh` copies all of
  it into `~/.local/lib/ferrite`). CI runs the package, and the video tests, on a clean
  Ubuntu with no GStreamer before it is published (T-337).
- macOS packages are checked the same way: nothing in the app may load a library from
  Homebrew or another place only the build machine has.

### Video

- **A video fed by a page (YouTube's way) now ends properly.** At the very end the
  element sometimes stopped and never said "ended". About one play in six (T-339).
- **Video no longer stops on a busy computer.** Under load, a stream could start before
  it was connected, or lose its connection while the sound output was being set up, and
  the video stopped (once this crashed the media engine). Now the stream waits for its
  connection, and a player that loses one starts again where it was (T-339).

### Page errors

- **YouTube's "window.cancelIdleCallback is not a function" is fixed.** A frame that a
  page sandboxes without scripts never got Ferrite's stand-ins for functions other
  browsers build in; YouTube calls them on such a frame. The page's own scripts in that
  frame still do not run (T-340).
- A rejected promise in the Console now names the script it came from: the browser's own
  exceptions carry a stack, as in other browsers (T-340).
- The "No valid entry type provided to observe()" warning (GitHub, Google, Amazon) now
  says which timings the page asked for (T-340).

### Still not done (honestly)

- **The browser can still feel slow.** Pages are drawn by the processor, not the graphics
  card (T-296, T-320). Run it with `FERRITE_PERF=1` and send the log, and we can see where
  the time goes on your machine.
- Google sign-in has not been confirmed end to end (T-267). YouTube playback cannot be
  checked on CI, because YouTube asks CI's machines to sign in.
- The builds are unsigned prototypes.
