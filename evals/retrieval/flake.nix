{
  description = "bilbo retrieval eval";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/2bd3427b41d10b8318383195efe502ed1baca6cd";

  outputs =
    { nixpkgs, ... }:
    let
      systems = [
        "aarch64-darwin"
        "aarch64-linux"
        "x86_64-darwin"
        "x86_64-linux"
      ];
      forAll = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      devShells = forAll (pkgs: {
        default = pkgs.mkShell {
          packages = with pkgs; [
            bash
            coreutils
            curl
            docker-client
            docker-compose
            findutils
            gnugrep
            gnutar
            zstd
          ];
          BILBO_EVAL = "retrieval";
          # nixpkgs' Compose looks for buildx only in ~/.docker/cli-plugins and here. Without it, Compose builds
          # with the classic builder, which ignores ADD --checksum.
          DOCKER_CLI_PLUGIN_DIRS = "${pkgs.docker-buildx}/libexec/docker/cli-plugins";
        };
      });
    };
}
