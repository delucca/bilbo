{
  description = "Notebooks, tasks and coding-agent sessions in one place";

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
      # Dev shell only: Bazel owns the build graph, and rules_apple needs the host Xcode, which Nix cannot supply.
      devShells = forAll (pkgs: {
        default = pkgs.mkShellNoCC {
          packages = with pkgs; [
            bazelisk
            # nixpkgs' bazelisk ships no bazel binary.
            (writeShellScriptBin "bazel" ''exec ${lib.getExe bazelisk} "$@"'')
          ];
        };
      });
    };
}
