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
      in {
        packages.spot = spot;

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

            # SPIN's pan.c is compiled with gcc on every query.
            gcc

            # bench/*.py use the standard library only.
            python3
          ];
        };
      });
}
