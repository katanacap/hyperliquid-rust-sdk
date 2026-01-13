{ pkgs, lib, config, inputs, ... }:

{
  # https://devenv.sh/packages/
  packages = [
      pkgs.curl
      pkgs.git
      pkgs.jq
      pkgs.llvm

      pkgs.just
      pkgs.tree

      ## Rust
      pkgs.rustup
      pkgs.sccache
      pkgs.cargo-outdated
      pkgs.cargo-nextest
      pkgs.cargo-audit
      
  ];

  # https://devenv.sh/languages/
  languages.nix.enable = true;
  languages.rust = {
    channel = "nightly";
    enable = true;
  };

  env.RUSTC_WRAPPER = "${pkgs.sccache}/bin/sccache";
  env.SCCACHE_CACHE_SIZE = "4G";
}