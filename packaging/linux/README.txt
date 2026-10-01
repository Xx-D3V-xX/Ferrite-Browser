Ferrite for Linux (x86_64)

  ./ferrite                      run it
  ./install.sh                   copy it into ~/.local (launcher entry + icon)

Needs a graphical session (X11 or Wayland) with OpenGL/EGL and these shared
libraries from your distribution: fontconfig, freetype, libxkbcommon, and
X11/Wayland client libraries. On Ubuntu/Debian:

  sudo apt install libfontconfig1 libfreetype6 libxkbcommon0 libegl1 \
       libgl1 libx11-6 libwayland-client0 libvulkan1 libudev1

Set your model key and options in ~/.ferrite/env.local (see the project README).
