{
  description = "iso-cc — rootless declarative sandbox sessions for Claude Code";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    crane.url = "github:ipetkov/crane";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, crane, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };

        # Pinned by flake.lock: the overlay revision decides the exact rust
        # version, so toolchain churn is bounded by lock updates only.
        rustToolchainFor = p: p.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" "clippy" ];
          targets = [ "x86_64-unknown-linux-musl" ];
        };
        craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchainFor;

        # cleanCargoSource 只保留 cargo 面（.rs/TOML/Lock）——18B 的 include_str!
        # 依赖 src/mark-uidrun.c（file-cap 助手源），必须显式放行，否则 nix 构建
        # 在编译期 No such file（本地 cargo 构建不暴露该缺口）。
        src = pkgs.lib.cleanSourceWith {
          src = ./.;
          filter = path: type:
            (pkgs.lib.hasSuffix "\.c" path) || (craneLib.filterCargoSources path type);
        };

        commonArgs = {
          inherit src;
          strictDeps = true;
        };

        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        iso-cc = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;
        });
      in
      {
        checks = {
          inherit iso-cc;

          clippy = craneLib.cargoClippy (commonArgs // {
            inherit cargoArtifacts;
            cargoClippyExtraArgs = "--all-targets -- --deny warnings";
          });

          fmt = craneLib.cargoFmt { inherit src; };

          nextest = craneLib.cargoNextest (commonArgs // {
            inherit cargoArtifacts;
            partitions = 1;
            partitionType = "count";
            cargoNextestPartitionsExtraArgs = "--no-tests=pass";
          });
        };

        packages.default = iso-cc;

        devShells.default = craneLib.devShell {
          checks = self.checks.${system};

          inputsFrom = [ iso-cc ];

          packages = with pkgs; [
            passt
            slirp4netns
            curl
            jq
            dnsutils
            nodejs
            python3
            iptables
          ];
        };
      });
}
