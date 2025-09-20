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
      pkgs.werf

      ## Rust
      pkgs.rustup
      pkgs.sccache
      pkgs.cargo-outdated
      pkgs.cargo-nextest
      pkgs.cargo-audit
      
  ] ++ lib.optionals pkgs.stdenv.isDarwin (with pkgs.darwin.apple_sdk; [
       frameworks.SystemConfiguration
       frameworks.Security
       frameworks.CoreFoundation
     ]);

  # https://devenv.sh/languages/
  languages.nix.enable = true;
  languages.rust = {
    channel = "nightly";
    enable = true;
  };

  env.RUSTC_WRAPPER = "${pkgs.sccache}/bin/sccache";
  env.SCCACHE_CACHE_SIZE = "4G";
  # env.RUSTFLAGS="-C target-cpu=native";
  # env.CARGO_TERM_COLOR = "always";
  # env.CARGO_TERM_MODE = "always";
  # languages.rust.mold.enable = true;

  # https://devenv.sh/processes/
  # processes.cargo-watch.exec = "cargo-watch";

  # services.clickhouse.enable = true;
  # services.clickhouse.httpPort = 8111;
  # services.clickhouse.port = 9111;
  
  # environment.etc = {
  #   # With changes from https://theorangeone.net/posts/calming-down-clickhouse/
  #   "clickhouse-server/config.d/custom.xml".source = lib.mkForce ./clickhouse-config.xml;
  #   "clickhouse-server/users.d/custom.xml".source = lib.mkForce ./clickhouse-users.xml;
  # };

  dotenv.enable = true;

  # env.CLICKHOUSE_ADDR = "127.0.0.1:9111";
  # env.DOCKERFILE_NAME = "aarch64.Dockerfile"; # for local development on mac aarch64
}