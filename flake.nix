{
  description = "Durable memory for coding agents";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-26.05-darwin";
    # Only the dev shell reads it, for cargo-dist 0.33.0 (nixpkgs 26.05 ships 0.31.0).
    nixpkgs-unstable.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    # Only the flake check that evaluates homeManagerModules.default reads it.
    home-manager = {
      url = "github:nix-community/home-manager/release-26.05";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      nixpkgs-unstable,
      home-manager,
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

      homeManagerModules.default =
        {
          config,
          lib,
          pkgs,
          ...
        }:
        let
          cfg = config.programs.bilbo;
          # The order bilbo setup writes them in (config::KEYS).
          keys = [
            "embedder.url"
            "embedder.model"
            "embedder.token_file"
            "embedder.token_env"
            "embedder.query_prefix"
            "embedder.min_similarity"
          ];
          # The same quoting rule as config::render in src/config.rs.
          quote =
            value:
            let
              needsQuotes =
                value == ""
                || lib.hasPrefix " " value
                || lib.hasSuffix " " value
                || lib.hasPrefix "\t" value
                || lib.hasSuffix "\t" value
                || lib.hasPrefix "\"" value
                || lib.hasInfix "\n" value
                || lib.hasInfix "\r" value;
            in
            if needsQuotes then
              "\"" + builtins.replaceStrings [ "\\" "\n" "\"" ] [ "\\\\" "\\n" "\\\"" ] value + "\""
            else
              builtins.replaceStrings [ "\\" ] [ "\\\\" ] value;
          present = lib.filter (key: cfg.settings.${key} != null) keys;
          text = lib.concatMapStrings (key: "${key} = ${quote cfg.settings.${key}}\n") (
            lib.filter (key: key != "embedder.query_prefix" || cfg.settings.${key} != "") present
          );
          flags = lib.escapeShellArgs (
            [ "--yes" ]
            ++ lib.optionals (cfg.claude != null) [
              "--claude"
              cfg.claude
            ]
            ++ lib.optionals (cfg.codex != null) [
              "--codex"
              cfg.codex
            ]
            ++ (
              if cfg.index.enable then
                [
                  "--index-every"
                  (toString cfg.index.every)
                ]
              else
                [ "--no-timer" ]
            )
          );
        in
        {
          options.programs.bilbo = {
            enable = lib.mkEnableOption "bilbo, durable memory for coding agents";
            package = lib.mkOption {
              type = lib.types.package;
              default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
              defaultText = lib.literalExpression "bilbo.packages.\${system}.default";
              description = "The bilbo package.";
            };
            storeRoot = lib.mkOption {
              type = lib.types.nullOr lib.types.str;
              default = null;
              example = "/Users/me/notes";
              description = "The store root, exported as BILBO_HOME. null keeps bilbo's default, $XDG_DATA_HOME/bilbo.";
            };
            settings = lib.mkOption {
              type = lib.types.submodule {
                options = lib.genAttrs keys (
                  key:
                  lib.mkOption {
                    type = lib.types.nullOr lib.types.str;
                    default = null;
                    description = "The config key ${key}, as the bilbo config spec defines it.";
                  }
                );
              };
              default = { };
              example = {
                "embedder.url" = "http://localhost:11434";
                "embedder.model" = "nomic-embed-text";
              };
              description = "Settings written to the bilbo config file.";
            };
            index = {
              enable = lib.mkOption {
                type = lib.types.bool;
                default = true;
                description = "Whether bilbo setup installs the index timer (it needs embedder.url).";
              };
              every = lib.mkOption {
                type = lib.types.ints.between 1 1440;
                default = 15;
                description = "Minutes between two runs of bilbo index.";
              };
            };
            claude = lib.mkOption {
              type = lib.types.nullOr lib.types.str;
              default = null;
              description = "The claude executable; null looks it up on the activation PATH, which rarely has it.";
            };
            codex = lib.mkOption {
              type = lib.types.nullOr lib.types.str;
              default = null;
              description = "The codex executable; null looks it up on the activation PATH, which rarely has it.";
            };
          };

          config = lib.mkIf cfg.enable {
            assertions = [
              {
                assertion = !(cfg.index.enable && cfg.settings."embedder.token_env" != null);
                message = "programs.bilbo: the index timer cannot read the variable in settings.\"embedder.token_env\"; set settings.\"embedder.token_file\" instead, or index.enable = false.";
              }
            ];
            home.packages = [ cfg.package ];
            home.sessionVariables = lib.mkIf (cfg.storeRoot != null) { BILBO_HOME = cfg.storeRoot; };
            xdg.configFile."bilbo/config".text =
              "# bilbo config, written by home-manager from programs.bilbo.settings\n" + text;
            home.activation.bilboSetup = lib.hm.dag.entryAfter [ "writeBoundary" "setupLaunchAgents" ] ''
              run env -u BILBO_HOME -u BILBO_CONFIG \
                ${
                  lib.optionalString (cfg.storeRoot != null) "BILBO_HOME=${lib.escapeShellArg cfg.storeRoot} "
                }XDG_CONFIG_HOME=${lib.escapeShellArg config.xdg.configHome} \
                XDG_DATA_HOME=${lib.escapeShellArg config.xdg.dataHome} \
                XDG_CACHE_HOME=${lib.escapeShellArg config.xdg.cacheHome} \
                XDG_STATE_HOME=${lib.escapeShellArg config.xdg.stateHome} \
                ${
                  if pkgs.stdenv.hostPlatform.isDarwin then
                    ''PATH="$PATH:/usr/bin:/bin"''
                  else
                    ''PATH="$PATH:${dirOf config.systemd.user.systemctlPath}" XDG_RUNTIME_DIR="''${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"''
                } \
                ${lib.getExe cfg.package} setup ${flags} \
                || warnEcho "bilbo setup reported a failed step; see its lines above"
            '';
          };
        };

      checks = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          hm =
            bilbo:
            home-manager.lib.homeManagerConfiguration {
              inherit pkgs;
              modules = [
                self.homeManagerModules.default
                {
                  home.username = "test";
                  home.homeDirectory = "/home/test";
                  home.stateVersion = "26.05";
                  programs.bilbo = bilbo;
                }
              ];
            };
          sample = hm {
            enable = true;
            settings = {
              "embedder.url" = "http://bagend:8081";
              "embedder.model" = "qwen3-embedding-0.6b";
              "embedder.query_prefix" = "Instruct: Given a question, retrieve notes that answer it\nQuery: ";
            };
            claude = "/opt/claude/bin/claude";
          };
          misspelled = hm {
            enable = true;
            settings."embeder.url" = "http://bagend:8081";
          };
          tokenEnv = hm {
            enable = true;
            settings = {
              "embedder.url" = "https://api.openai.com";
              "embedder.model" = "text-embedding-3-small";
              "embedder.token_env" = "OPENAI_API_KEY";
            };
          };
          disabled = hm { enable = false; };
          rooted = hm {
            enable = true;
            storeRoot = "/data/my notes/bilbo";
            index.enable = false;
          };
          rootedActivation = rooted.config.home.activation.bilboSetup.data;
          configText = sample.config.xdg.configFile."bilbo/config".text;
          activation = sample.config.home.activation.bilboSetup.data;
          fails = c: !(builtins.tryEval c.activationPackage.drvPath).success;
        in
        {
          default = self.packages.${system}.bilbo;
          home-manager-module =
            assert
              configText == ''
                # bilbo config, written by home-manager from programs.bilbo.settings
                embedder.url = http://bagend:8081
                embedder.model = qwen3-embedding-0.6b
                embedder.query_prefix = "Instruct: Given a question, retrieve notes that answer it\nQuery: "
              '';
            assert lib.hasInfix "/bin/bilbo setup --yes --claude /opt/claude/bin/claude --index-every 15"
              activation;
            assert lib.hasInfix "BILBO_HOME=${lib.escapeShellArg "/data/my notes/bilbo"} " rootedActivation;
            assert lib.hasInfix " --no-timer" rootedActivation;
            assert rooted.config.home.sessionVariables.BILBO_HOME == "/data/my notes/bilbo";
            assert (builtins.tryEval sample.activationPackage.drvPath).success;
            assert fails misspelled;
            assert fails tokenEnv;
            assert !(disabled.config.home.activation ? bilboSetup);
            assert !(disabled.config.xdg.configFile ? "bilbo/config");
            pkgs.writeText "bilbo-home-manager-module" (builtins.unsafeDiscardStringContext activation);
        }
      );

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
