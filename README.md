
![Remcord](https://github.com/user-attachments/assets/f7b007d4-44fa-4c88-96e4-0a448b568b5d)

# Features

- **Standalone client**

   Remcord is built as a standalone client and doesn't rely on the original Discord client in any way.

- **Various mods built-in**

   Enjoy [Vencord](https://github.com/Vendicated/Vencord), [Equicord](https://github.com/Equicord/Equicord), [Shelter](https://github.com/uwu/shelter) and their many features, or have a more vanilla experience, it's your choice!

- **Themes**

   Remcord natively supports theming of the entire app, you can easily import BetterDiscord themes and manage them

- **Made for Privacy™**

   Remcord automatically blocks all of Discord's trackers; even without any client mods, you can feel safe and secure!

- **Supports Rich Presence**

   Unlike other clients, Remcord supports rich presence (game activity) out of the box thanks to [arRPC](https://arrpc.openasar.dev).

- **Mobile support**

   Remcord has **experimental** mobile support for phones running Linux such as the PinePhone. While this is still far from an ideal solution, we're slowly trying to improve it.

- **Much more stable**

   The production Windows app is the Rust native WebView2 shell (built + signed in CI). The Electron sources are stale legacy code that is no longer built, signed, or released — kept as reference only.

- **Cross-platform support!**

   Remcord was originally created for AArch64 Linux devices since Discord doesn't support them. The Electron sources are now stale legacy code that is no longer built, signed, or released; the Rust native app is the production Windows app.
  
# How to run/install it?

## Packaging status

Packaged distributions for the renamed product follow under new identifiers; the previous generation's marketplace listings stay with the previous generation and are not renamed in place.

### Windows

Build the installer from source with `native/installer/build-installer.ps1` (requires Inno Setup 6), producing `Remcord-Setup-<version>.exe`.

### Linux

Build distributable targets from source with `pnpm run package` (see `electron-builder.ts` for the configured AppImage, deb, rpm, tar.gz, and Flatpak metadata).

### macOS

Build from source with `pnpm run package` (see `electron-builder.ts`).

### Compiling

 Alternatively, you can run Remcord from source ([NodeJS v26 and newer](https://nodejs.dev) and [pnpm](https://pnpm.io/installation#using-npm)) are required:

 1. Clone the project repository
 2. Run `pnpm install` to install dependencies
 3. Build with `pnpm run build`
 4. Compile/Package with `pnpm run package`
# FAQ

## Do you have a support Discord?

[![Discord Server](https://dcbadge.vercel.app/api/server/TnhxcqynZ2)](https://discord.gg/TnhxcqynZ2)

## Will I get banned for using this?

- You are breaking [Discord ToS](https://discord.com/terms#software-in-discord%E2%80%99s-services) by using Remcord, but no one has been banned from using it or any of the client mods included.

## How can I access the settings?

- Open Discord settings and there should be a button `Remcord Settings` button with a white Discord icon, you can also right click on the tray icon and click `Open Settings`

## How does this work?

- The production Windows app is the Rust native WebView2 shell, which wraps the official web app. The Electron packaging described in older history is stale legacy code that is no longer built, signed, or released. While this approach may seem familiar, our focus is on delivering a truly customized and enhanced experience. Unlike many others, we provide seamless integration for loading themes and mods without the need for installers or injectors. You can easily enable transparency effects and adopt Windows' Fluent Design, offering a modern and sleek interface. Though it's fundamentally a web wrapper, we have implemented numerous optimizations and patches to ensure a smooth and tailored experience for you.

## Does Remcord have a portable mode for windows?

- Yes it does! Simply add a folder called "remcord-data" in the folder where your remcord executable is located and start Remcord. Make sure to download the archive/zip file.

## Where can I find the source code?

- The source code ships with the project repository.

## Where can I translate this?

- Translations are done using our [Weblate page](https://hosted.weblate.org/projects/armcord/).

# Credits

- [Remcord UI design, branding, and a few features](https://github.com/kckarnige)
- [OpenAsar](https://github.com/GooseMod/OpenAsar)
- [arRPC (for Rich Presence)](https://github.com/OpenAsar/arrpc)
- [electron-builder](https://electron.build)
  
Discord is trademark of Discord Inc. Remcord is not affiliated with or endorsed by Discord Inc.
Remcord is not affiliated with or endorsed by ARM Limited.
