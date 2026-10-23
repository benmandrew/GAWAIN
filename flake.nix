{
  description = "GAWAIN — specification weakening against a model by genetic search";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          # nuXmv is free for non-commercial use but not redistributable.
          config.allowUnfreePredicate = p: nixpkgs.lib.getName p == "nuxmv";
        };

        # Spot (ltlfilt, ltl2tgba, autfilt, ltlgrind, randltl) is not in
        # nixpkgs. The oracle's batching and its workaround for the
        # `ltlfilt --imply` bug are measured against 2.15.1, so pin it.
        # The Python bindings are not used.
        spot = pkgs.stdenv.mkDerivation (finalAttrs: {
          pname = "spot";
          version = "2.15.1";
          src = pkgs.fetchurl {
            url = "https://www.lrde.epita.fr/dload/spot/spot-${finalAttrs.version}.tar.gz";
            hash = "sha256-ZQE6Lt8/MUhU12GYiBRfUsjdNr/SeJTZ25snLZoWzks=";
          };
          configureFlags = [ "--disable-python" "--disable-static" ];
          enableParallelBuilding = true;
        });

        # Ganak, the model counter behind the model-counting objective, is
        # not in nixpkgs. PEREDUR (cmake/ganak.cmake) uses the upstream
        # release binary, so this does too, at the same version. The binary
        # is statically linked, so it needs no patchelf. Only x86_64 Linux
        # is pinned; elsewhere the shell has no ganak and its tests skip.
        ganak = pkgs.stdenvNoCC.mkDerivation (finalAttrs: {
          pname = "ganak";
          version = "2.5.3";
          src = pkgs.fetchurl {
            url = "https://github.com/meelgroup/ganak/releases/download/release/${finalAttrs.version}/ganak-linux-amd64.zip";
            hash = "sha256-IpCDUV5X/k4lxcf2v7Cw6E+LdMJ2Rk1eLy3dDKHvcUA=";
          };
          nativeBuildInputs = [ pkgs.unzip ];
          unpackPhase = "unzip $src ganak";
          installPhase = "install -Dm755 ganak $out/bin/ganak";
          dontStrip = true;
          meta.platforms = [ "x86_64-linux" ];
        });

        # TAPAAL's verifypn, the MCC back end (src/oracle/tapaal.rs), is not
        # in nixpkgs, and its source build needs gcc 16 and its own Spot.
        # The TAPAAL 4.0.4 bundle ships it as a static x86-64 binary
        # (verifypn 4.4.2, GPL-3.0-or-later); install that as `verifypn`.
        verifypn = pkgs.stdenvNoCC.mkDerivation {
          pname = "verifypn";
          version = "4.4.2";
          src = pkgs.fetchurl {
            url = "https://download.tapaal.net/tapaal/tapaal-4.0/tapaal-4.0.4-linux64.zip";
            hash = "sha256-gVqruXofCVyXwnktxEpWlwJLUHbVomuDqRo+kUdB2PM=";
          };
          nativeBuildInputs = [ pkgs.unzip ];
          installPhase = ''
            install -Dm755 bin/verifypn64 $out/bin/verifypn
          '';
          meta.platforms = [ "x86_64-linux" ];
        };
      in {
        packages.spot = spot;
        packages.ganak = ganak;
        packages.verifypn = verifypn;

        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            # Rust toolchain; CI's check job needs rustfmt and clippy.
            cargo
            rustc
            rustfmt
            clippy

            # Model checkers and LTL tools the oracle runs as processes.
            # nixpkgs builds SPIN from upstream source, whose makefile sets
            # -DNXT, so it accepts X (Ubuntu's package does not).
            spot
            spin
            nuxmv
          ] ++ pkgs.lib.optionals (system == "x86_64-linux") [ ganak verifypn ] ++ (with pkgs; [

            # SPIN's pan.c is compiled with gcc on every query.
            gcc

            # bench/*.py use the standard library only.
            python3
          ]);
        };
      });
}
