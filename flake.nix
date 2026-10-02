{
  description = "Durable memory for coding agents";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-26.05-darwin";
    # Only the dev shell reads it, for cargo-dist 0.33.0 (nixpkgs 26.05 ships 0.31.0).
    nixpkgs-unstable.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
  };

  outputs =
    {
      self,
      nixpkgs,
      nixpkgs-unstable,
    }:
    let
      lib = nixpkgs.lib;
      forAllSystems = lib.genAttrs [
        "aarch64-darwin"
        "x86_64-linux"
        "aarch64-linux"
      ];
      version = (lib.importTOML ./Cargo.toml).package.version;
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = self.packages.${system}.bilbo;
          bilbo = pkgs.rustPlatform.buildRustPackage {
            pname = "bilbo";
            inherit version;
            src = lib.fileset.toSource {
              root = ./.;
              fileset = lib.fileset.unions [
                ./Cargo.toml
                ./Cargo.lock
                ./src
                ./tests
                ./plugins
                ./.claude-plugin/marketplace.json
                ./.agents/plugins/marketplace.json
              ];
            };
            cargoLock.lockFile = ./Cargo.lock;
            # The fake embedder in tests/common listens on 127.0.0.1.
            __darwinAllowLocalNetworking = true;
            # .github is not in src; the verify job runs this test.
            checkFlags = [
              "--skip"
              "every_action_is_pinned_by_sha"
            ];
            postInstall = ''
              mkdir -p $out/share/bilbo/.claude-plugin $out/share/bilbo/.agents/plugins
              cp -r plugins $out/share/bilbo/plugins
              cp .claude-plugin/marketplace.json $out/share/bilbo/.claude-plugin/
              cp .agents/plugins/marketplace.json $out/share/bilbo/.agents/plugins/
            '';
            meta = {
              description = "Durable memory for coding agents";
              homepage = "https://github.com/delucca/bilbo";
              license = lib.licenses.asl20;
              mainProgram = "bilbo";
            };
          };
        }
      );

      checks = forAllSystems (system: {
        default = self.packages.${system}.bilbo;
      });

      devShells = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = pkgs.mkShell {
            packages = [
              pkgs.cargo
              pkgs.rustc
              pkgs.clippy
              pkgs.rustfmt
              nixpkgs-unstable.legacyPackages.${system}.cargo-dist
            ];
          };
        }
      );
    };
}
