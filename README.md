# Nyx Installer

NyxOS's installer: a GTK app (Nyx Design 2.0) on the live USB, and `nyx-install`, the
same without a window. It erases a disk or installs beside Windows in free space, with
LUKS2 encryption, Btrfs subvolumes (`@`, `@home`, `@nix`, `@log`, `@swap`) and a swap
file for hibernating, then writes the machine's flake (`/etc/nyxos/flake`, NyxOS on a
release channel) and installs it. Beside Windows, NyxOS gets its own EFI partition and
nothing of Windows' is written to; Limine starts Windows Boot Manager.

- `crates/nyx-install`: disks, plans and the steps that carry them out (no UI)
