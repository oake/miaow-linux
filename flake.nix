{
  description = "miaow — native GNOME one-to-one room calls";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    crane.url = "github:ipetkov/crane";
  };

  outputs = { self, nixpkgs, crane }:
    let
      systems = [ "x86_64-linux" ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in {
      packages = forAllSystems (system:
        let
          pkgs = import nixpkgs {
            inherit system;
            config.allowUnfree = true;
          };
          craneLib = crane.mkLib pkgs;
          # Keep Cargo inputs plus runtime assets. `commonCargoSources` makes
          # the dependency derivation insensitive to ordinary source edits,
          # while the full package still notices CSS/desktop/schema changes.
          src = pkgs.lib.fileset.toSource {
            root = ./.;
            fileset = pkgs.lib.fileset.unions [
              (craneLib.fileset.commonCargoSources ./.)
              ./data
              ./gnome-shell-extension
            ];
          };
          webrtc = pkgs.fetchzip {
            url = "https://github.com/livekit/rust-sdks/releases/download/webrtc-51ef663/webrtc-linux-x64-release.zip";
            hash = "sha256-Cho7WsJBhkUs0ZCSXwj4zCXIn7NYNHbHo4UGVU/0jwY=";
          };
          commonArgs = {
            inherit src;
            pname = "miaow";
            version = "0.1.0";
            strictDeps = true;

            nativeBuildInputs = with pkgs; [
              pkg-config
              wrapGAppsHook4
              glib
              clang
              lld
            ];

            buildInputs = with pkgs; [
              gtk4
              libadwaita
              pipewire
              libpulseaudio
              alsa-lib
              libGL
              libjpeg_turbo
              libva
              libdrm
              udev
              cudaPackages.cuda_cudart
            ];

            LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
            LK_CUSTOM_WEBRTC = webrtc;
            CUDA_HOME = "${pkgs.cudaPackages.cuda_cudart}";

          };

          # This derivation depends only on Cargo manifests/lock data. Source
          # edits reuse its complete GTK/LiveKit/WebRTC target directory.
          cargoArtifacts = craneLib.buildDepsOnly commonArgs;
        in {
          default = craneLib.buildPackage (commonArgs // {
            inherit cargoArtifacts;
            # Keep the interactive package path quick. Tests remain available
            # as `nix build .#tests` and through `nix flake check`.
            doCheck = false;

            preFixup = ''
              gappsWrapperArgs+=(
                # WebRTC loads these backends with dlopen, so the ELF closure
                # scanner cannot discover them on its own.
                --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath [
                  pkgs.libpulseaudio
                  pkgs.alsa-lib
                  pkgs.pipewire
                  pkgs.libva
                  pkgs.libdrm
                ]}
                --prefix LD_LIBRARY_PATH : /run/opengl-driver/lib
              )
            '';

            postInstall = ''
              install -Dm644 data/ke.oa.miaow.desktop \
                $out/share/applications/ke.oa.miaow.desktop
              install -Dm644 data/ke.oa.miaow.gschema.xml \
                $out/share/glib-2.0/schemas/ke.oa.miaow.gschema.xml
              install -Dm644 data/ke.oa.miaow.svg \
                $out/share/icons/hicolor/scalable/apps/ke.oa.miaow.svg
              install -d $out/share/gnome-shell/extensions/miaow@oa.ke
              install -m644 gnome-shell-extension/extension.js \
                gnome-shell-extension/metadata.json \
                $out/share/gnome-shell/extensions/miaow@oa.ke/
              install -Dm644 data/ke.oa.miaow.gschema.xml \
                $out/share/gnome-shell/extensions/miaow@oa.ke/schemas/ke.oa.miaow.gschema.xml
              glib-compile-schemas \
                $out/share/gnome-shell/extensions/miaow@oa.ke/schemas
              glib-compile-schemas $out/share/glib-2.0/schemas
            '';

            passthru.extensionUuid = "miaow@oa.ke";

            meta = {
              description = "Native GNOME one-to-one room-call client";
              homepage = "https://oa.ke";
              license = pkgs.lib.licenses.mit;
              mainProgram = "miaow";
              platforms = systems;
            };
          });

          tests = craneLib.cargoTest (commonArgs // {
            inherit cargoArtifacts;
          });

          clippy = craneLib.cargoClippy (commonArgs // {
            inherit cargoArtifacts;
            cargoClippyExtraArgs = "--all-targets -- --deny warnings";
          });

          format = craneLib.cargoFmt { inherit src; };
        });

      checks = forAllSystems (system: {
        inherit (self.packages.${system}) tests clippy format;
      });

      apps = forAllSystems (system: {
        default = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/miaow";
          meta = {
            description = "Native GNOME one-to-one room-call client";
          };
        };
      });

      devShells = forAllSystems (system:
        let pkgs = import nixpkgs { inherit system; };
        in {
          default = pkgs.mkShell {
            inputsFrom = [ self.packages.${system}.default ];
            packages = with pkgs; [ rustc cargo rustfmt clippy rust-analyzer ];
            LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
            inherit (self.packages.${system}.default) LK_CUSTOM_WEBRTC CUDA_HOME;
          };
        });
    };
}
