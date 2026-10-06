{
  description = "Durable memory for coding agents";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-26.05-darwin";
    # Only the dev shell reads it, for cargo-dist 0.33.0 (nixpkgs 26.05 ships 0.31.0).
    nixpkgs-unstable.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    # Builds the dependencies as their own derivation, which CI caches.
    crane.url = "github:ipetkov/crane/v0.24.0";
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
      crane,
      home-manager,
    }:
    let
      lib = nixpkgs.lib;
      forAllSystems = lib.genAttrs [
        "aarch64-darwin"
        "x86_64-linux"
        "aarch64-linux"
      ];
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          craneLib = crane.mkLib pkgs;
          common = {
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
            outputHashes."git+https://github.com/algesten/ureq?rev=0ebb046e1cf269592f9edb3a55f489d508f47a1d#0ebb046e1cf269592f9edb3a55f489d508f47a1d" =
              "sha256-BIj3Wb8r0WG4UPB8hSlHdjBp1u8JbLUHbDRdRNv79bc=";
          };
        in
        {
          default = self.packages.${system}.bilbo;
          bilbo = craneLib.buildPackage (
            common
            // {
              cargoArtifacts = craneLib.buildDepsOnly common;
              # The fake embedder in tests/common listens on 127.0.0.1.
              __darwinAllowLocalNetworking = true;
              # .github is not in src; the verify job runs this test.
              cargoTestExtraArgs = "-- --skip every_action_is_pinned_by_sha";
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
            }
          );
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
          localUrl = "http://127.0.0.1:${toString cfg.localEmbedder.port}";
          localModel = "qwen3-embedding-0.6b";
          # The order bilbo setup writes them in (config::KEYS).
          keys = [
            "embedder.url"
            "embedder.model"
            "embedder.token_file"
            "embedder.token_env"
            "embedder.query_prefix"
            "embedder.min_similarity"
            "digest.enable"
            "digest.min_similarity"
            "digest.log"
            "history.keep_days"
            "sync.poll_seconds"
            "sync.stale_days"
          ];
          # A scope key: scope.default, or scope.<name>.<sub> with the topic's grammar for <name>.
          isScopeKey =
            key:
            key == "scope.default"
            || (
              let
                parts = builtins.match "scope\\.([a-z0-9]+(-[a-z0-9]+)*)\\.(sync|embedder|paths|marks)" key;
              in
              parts != null && builtins.head parts != "default"
            );
          # The same quoting rule as config::render in src/shared/config.rs.
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
          badKeys = lib.filter (key: !(lib.elem key keys) && !isScopeKey key) (lib.attrNames cfg.settings);
          scopeKeys = lib.sort builtins.lessThan (lib.filter isScopeKey (lib.attrNames cfg.settings));
          present = lib.filter (key: cfg.settings.${key} != null) (keys ++ scopeKeys);
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
            ++ lib.optionals (!cfg.watch.enable) [ "--no-watch" ]
            ++ lib.optionals cfg.localEmbedder.enable [
              "--embedder-local"
              "--embedder-port"
              (toString cfg.localEmbedder.port)
              "--llama-server"
              cfg.localEmbedder.llamaServer
            ]
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
                freeformType = lib.types.attrsOf (lib.types.nullOr lib.types.str);
                options = lib.genAttrs keys (
                  key:
                  lib.mkOption {
                    type = lib.types.nullOr lib.types.str;
                    default = null;
                    description = "The config key ${key}; see https://github.com/delucca/bilbo/blob/main/docs/reference/configuration.md.";
                  }
                );
              };
              default = { };
              example = {
                "embedder.url" = "http://localhost:11434";
                "embedder.model" = "nomic-embed-text";
              };
              description = "Settings written to the bilbo config file: the fixed keys (embedder, digest, history and sync), and scope.<name>.sync|embedder|paths|marks and scope.default.";
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
            watch = {
              enable = lib.mkOption {
                type = lib.types.bool;
                default = true;
                description = "Whether bilbo setup installs the watcher, the login service that records note history.";
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
            localEmbedder = {
              enable = lib.mkEnableOption "the local embedder: bilbo downloads its pinned model and runs llama-server as a login service";
              port = lib.mkOption {
                type = lib.types.ints.between 1024 65535;
                default = 8737;
                description = "The port llama-server listens on, on 127.0.0.1.";
              };
              llamaServer = lib.mkOption {
                type = lib.types.str;
                default = lib.getExe' pkgs.llama-cpp "llama-server";
                defaultText = lib.literalExpression ''lib.getExe' pkgs.llama-cpp "llama-server"'';
                description = "The llama-server executable the service runs.";
              };
            };
          };

          config = lib.mkIf cfg.enable {
            programs.bilbo.settings = lib.mkIf cfg.localEmbedder.enable {
              "embedder.url" = lib.mkDefault localUrl;
              "embedder.model" = lib.mkDefault localModel;
              "embedder.query_prefix" =
                lib.mkDefault "Instruct: Given a question, retrieve notes that answer it\nQuery: ";
            };
            assertions = [
              {
                assertion = badKeys == [ ];
                message = "programs.bilbo: unknown settings ${lib.concatStringsSep ", " badKeys}; a key is one of ${lib.concatStringsSep ", " keys}, scope.default or scope.<name>.sync|embedder|paths|marks.";
              }
              {
                assertion =
                  !cfg.localEmbedder.enable
                  || (cfg.settings."embedder.url" == localUrl && cfg.settings."embedder.model" == localModel);
                message = "programs.bilbo: localEmbedder.enable serves ${localModel} at ${localUrl}; leave settings.\"embedder.url\" and settings.\"embedder.model\" unset, or set them to those values.";
              }
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

      nixosModules.relay =
        {
          config,
          lib,
          pkgs,
          ...
        }:
        let
          cfg = config.services.bilbo-relay;
        in
        {
          options.services.bilbo-relay = {
            enable = lib.mkEnableOption "the bilbo relay, the server that syncs a user's devices";
            package = lib.mkOption {
              type = lib.types.package;
              default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
              defaultText = lib.literalExpression "bilbo.packages.\${system}.default";
              description = "The bilbo package.";
            };
            owners = lib.mkOption {
              type = lib.types.listOf lib.types.str;
              default = [ ];
              example = [ "yb4b-5aju-v6zb-x2nm-nc5x-ompf" ];
              description = "The owner fingerprints the relay admits, one --owner each. It needs at least one.";
            };
            listen = lib.mkOption {
              type = lib.types.str;
              default = "127.0.0.1:8738";
              description = "The address:port the relay listens on, plain HTTP; put a TLS proxy in front of a non-loopback address.";
            };
            maxScopes = lib.mkOption {
              type = lib.types.nullOr lib.types.ints.positive;
              default = null;
              description = "The most valid scopes the relay holds per owner; null keeps the relay's default.";
            };
            maxScopeMb = lib.mkOption {
              type = lib.types.nullOr lib.types.ints.positive;
              default = null;
              description = "The most MiB one scope may hold; null keeps the relay's default.";
            };
            maxObjectMb = lib.mkOption {
              type = lib.types.nullOr lib.types.ints.positive;
              default = null;
              description = "The most MiB one object may hold; null keeps the relay's default.";
            };
          };

          config = lib.mkIf cfg.enable {
            assertions = [
              {
                assertion = cfg.owners != [ ];
                message = "services.bilbo-relay.owners must hold at least one owner fingerprint; the relay admits no scope without one.";
              }
            ];
            systemd.services.bilbo-relay = {
              description = "bilbo relay";
              wantedBy = [ "multi-user.target" ];
              after = [ "network.target" ];
              serviceConfig = {
                ExecStart = lib.escapeShellArgs (
                  [
                    (lib.getExe cfg.package)
                    "relay"
                    "--data"
                    "/var/lib/bilbo-relay"
                  ]
                  ++ lib.concatMap (owner: [
                    "--owner"
                    owner
                  ]) cfg.owners
                  ++ [
                    "--listen"
                    cfg.listen
                  ]
                  ++ lib.optionals (cfg.maxScopes != null) [
                    "--max-scopes"
                    (toString cfg.maxScopes)
                  ]
                  ++ lib.optionals (cfg.maxScopeMb != null) [
                    "--max-scope-mb"
                    (toString cfg.maxScopeMb)
                  ]
                  ++ lib.optionals (cfg.maxObjectMb != null) [
                    "--max-object-mb"
                    (toString cfg.maxObjectMb)
                  ]
                );
                Restart = "on-failure";
                RestartSec = 10;
                DynamicUser = true;
                StateDirectory = "bilbo-relay";
                StateDirectoryMode = "0700";
                UMask = "0077";
                ProtectSystem = "strict";
                ProtectHome = true;
                PrivateTmp = true;
                NoNewPrivileges = true;
                RestrictAddressFamilies = [
                  "AF_INET"
                  "AF_INET6"
                ];
                CapabilityBoundingSet = "";
                SystemCallFilter = [ "@system-service" ];
                SystemCallArchitectures = "native";
                ProtectKernelTunables = true;
                ProtectKernelModules = true;
                ProtectControlGroups = true;
                RestrictNamespaces = true;
                RestrictRealtime = true;
                LockPersonality = true;
                MemoryDenyWriteExecute = true;
                ProtectProc = "invisible";
              };
            };
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
              "embedder.url" = "http://embedder.example:8081";
              "embedder.model" = "qwen3-embedding-0.6b";
              "embedder.query_prefix" = "Instruct: Given a question, retrieve notes that answer it\nQuery: ";
            };
            claude = "/opt/claude/bin/claude";
          };
          misspelled = hm {
            enable = true;
            settings."embeder.url" = "http://embedder.example:8081";
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
          digestOnly = hm {
            enable = true;
            settings."digest.enable" = "off";
            settings."digest.log" = "on";
          };
          scoped = hm {
            enable = true;
            settings = {
              "scope.work.paths" = "~/Developer/acme";
              "scope.work.embedder" = "local";
              "scope.default" = "work";
              "history.keep_days" = "30";
            };
          };
          synced = hm {
            enable = true;
            settings = {
              "scope.personal.sync" = "file:///Users/a/Sync/bilbo";
              "sync.poll_seconds" = "60";
            };
          };
          badScope = hm {
            enable = true;
            settings."scope.work.colour" = "red";
          };
          badScopeName = hm {
            enable = true;
            settings."scope.default.sync" = "off";
          };
          local = hm {
            enable = true;
            localEmbedder.enable = true;
          };
          localElsewhere = hm {
            enable = true;
            localEmbedder.enable = true;
            settings."embedder.url" = "http://embedder.example:8081";
          };
          localOtherModel = hm {
            enable = true;
            localEmbedder.enable = true;
            settings."embedder.model" = "nomic-embed-text";
          };
          localConfig = local.config.xdg.configFile."bilbo/config".text;
          localActivation = builtins.unsafeDiscardStringContext local.config.home.activation.bilboSetup.data;
          llamaServer = builtins.unsafeDiscardStringContext (lib.getExe' pkgs.llama-cpp "llama-server");
          rooted = hm {
            enable = true;
            storeRoot = "/data/my notes/bilbo";
            index.enable = false;
            watch.enable = false;
            settings."history.keep_days" = "30";
          };
          rootedActivation = rooted.config.home.activation.bilboSetup.data;
          rootedConfig = rooted.config.xdg.configFile."bilbo/config".text;
          configText = sample.config.xdg.configFile."bilbo/config".text;
          activation = sample.config.home.activation.bilboSetup.data;
          fails = c: !(builtins.tryEval c.activationPackage.drvPath).success;
          relayHost =
            relay:
            nixpkgs.lib.nixosSystem {
              system = "x86_64-linux";
              modules = [
                self.nixosModules.relay
                {
                  fileSystems."/" = {
                    device = "none";
                    fsType = "tmpfs";
                  };
                  boot.loader.grub.enable = false;
                  system.stateVersion = "26.05";
                  services.bilbo-relay = relay;
                }
              ];
            };
          relayOwner = "yb4b-5aju-v6zb-x2nm-nc5x-ompf";
          relayService =
            (relayHost {
              enable = true;
              owners = [ relayOwner ];
            }).config.systemd.services.bilbo-relay.serviceConfig;
          relayLimited =
            (relayHost {
              enable = true;
              owners = [ relayOwner ];
              maxScopes = 4;
              maxObjectMb = 8;
            }).config.systemd.services.bilbo-relay.serviceConfig;
          relayFailed = lib.filter (a: !a.assertion) (relayHost {
            enable = true;
            owners = [ ];
          }).config.assertions;
          relayBin = builtins.unsafeDiscardStringContext (lib.getExe self.packages.x86_64-linux.default);
        in
        {
          default = self.packages.${system}.bilbo;
          home-manager-module =
            assert
              configText == ''
                # bilbo config, written by home-manager from programs.bilbo.settings
                embedder.url = http://embedder.example:8081
                embedder.model = qwen3-embedding-0.6b
                embedder.query_prefix = "Instruct: Given a question, retrieve notes that answer it\nQuery: "
              '';
            assert lib.hasInfix "/bin/bilbo setup --yes --claude /opt/claude/bin/claude --index-every 15"
              activation;
            assert lib.hasInfix "BILBO_HOME=${lib.escapeShellArg "/data/my notes/bilbo"} " rootedActivation;
            assert lib.hasInfix " --no-timer --no-watch" rootedActivation;
            assert !(lib.hasInfix "--no-watch" activation);
            assert
              rootedConfig == ''
                # bilbo config, written by home-manager from programs.bilbo.settings
                history.keep_days = 30
              '';
            assert rooted.config.home.sessionVariables.BILBO_HOME == "/data/my notes/bilbo";
            assert (builtins.tryEval sample.activationPackage.drvPath).success;
            assert fails misspelled;
            assert fails badScope;
            assert fails badScopeName;
            assert
              scoped.config.xdg.configFile."bilbo/config".text == ''
                # bilbo config, written by home-manager from programs.bilbo.settings
                history.keep_days = 30
                scope.default = work
                scope.work.embedder = local
                scope.work.paths = ~/Developer/acme
              '';
            assert (builtins.tryEval scoped.activationPackage.drvPath).success;
            assert
              synced.config.xdg.configFile."bilbo/config".text == ''
                # bilbo config, written by home-manager from programs.bilbo.settings
                sync.poll_seconds = 60
                scope.personal.sync = file:///Users/a/Sync/bilbo
              '';
            assert (builtins.tryEval synced.activationPackage.drvPath).success;
            assert fails tokenEnv;
            assert !(disabled.config.home.activation ? bilboSetup);
            assert
              localConfig == ''
                # bilbo config, written by home-manager from programs.bilbo.settings
                embedder.url = http://127.0.0.1:8737
                embedder.model = qwen3-embedding-0.6b
                embedder.query_prefix = "Instruct: Given a question, retrieve notes that answer it\nQuery: "
              '';
            assert lib.hasInfix
              "/bin/bilbo setup --yes --index-every 15 --embedder-local --embedder-port 8737 --llama-server ${llamaServer}"
              localActivation;
            assert (builtins.tryEval local.activationPackage.drvPath).success;
            assert fails localElsewhere;
            assert fails localOtherModel;
            assert !(disabled.config.xdg.configFile ? "bilbo/config");
            assert
              digestOnly.config.xdg.configFile."bilbo/config".text == ''
                # bilbo config, written by home-manager from programs.bilbo.settings
                digest.enable = off
                digest.log = on
              '';
            assert (builtins.tryEval digestOnly.activationPackage.drvPath).success;
            pkgs.writeText "bilbo-home-manager-module" (builtins.unsafeDiscardStringContext activation);
          nixos-relay-module =
            assert
              builtins.unsafeDiscardStringContext relayService.ExecStart
              == "${relayBin} relay --data /var/lib/bilbo-relay --owner ${relayOwner} --listen 127.0.0.1:8738";
            assert relayService.StateDirectoryMode == "0700";
            assert relayService.UMask == "0077";
            assert lib.hasInfix " --max-scopes 4 --max-object-mb 8" (
              builtins.unsafeDiscardStringContext relayLimited.ExecStart
            );
            assert !(lib.hasInfix "--max-scope-mb" (builtins.unsafeDiscardStringContext relayLimited.ExecStart));
            assert lib.length relayFailed == 1;
            assert lib.hasInfix "services.bilbo-relay.owners" (lib.head relayFailed).message;
            pkgs.writeText "bilbo-nixos-relay-module" (builtins.unsafeDiscardStringContext relayService.ExecStart);
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
