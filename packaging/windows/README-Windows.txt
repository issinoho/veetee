veetee for Windows
==================

Unzip anywhere and run bin\veetee.exe. Windows 10 version 1809 or later is
needed for local command windows (the pseudo console).

  bin\veetee.exe                          Command Prompt (%COMSPEC%)
  bin\veetee.exe --telnet vms1            Telnet
  bin\veetee.exe --ssh system@vms1        SSH, through Windows' OpenSSH client
  bin\veetee.exe --serial COM3            serial line, 9600 8N1 XON/XOFF
  bin\veetee.exe --model vt525 --telnet vms1

Run bin\veetee.exe --help from a Command Prompt for every option.

The keyboard map you save from the Keyboard Map window is kept in
%LOCALAPPDATA%\veetee\keymap.toml.

veetee draws with OpenGL; the graphics driver must support OpenGL 3.3.

The bundled GTK, libadwaita and other libraries are listed with their
licences in the licenses folder. They are unmodified MSYS2 builds; their
sources are available from https://packages.msys2.org/.
