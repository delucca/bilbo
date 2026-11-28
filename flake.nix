{
  description = "Notebooks, tasks and coding-agent sessions in one place";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/619c39f6eac5d1ed6b8f073a9414baf7034b2356";
    # Only the flake check that evaluates homeManagerModules.default reads it.
    home-manager = {
      url = "github:nix-community/home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      home-manager,
    }:
    let
      lib = nixpkgs.lib;
      systems = [
        "aarch64-darwin"
        "aarch64-linux"
        "x86_64-linux"
      ];
      forAll = f: lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});

      version = (fromTOML (builtins.readFile ./Cargo.toml)).workspace.package.version;

      src = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          ./Cargo.toml
          ./Cargo.lock
          ./apps/cli
          ./libs
          ./tools/rust
          ./tests/e2e/cli
          ./docs
          ./plugins
          ./.claude-plugin/marketplace.json
          ./.agents/plugins/marketplace.json
        ];
      };

      # The sandbox has no /usr/bin/plutil, which these two tests run on the units they render.
      skippedInTheSandbox = [
        "the_launchd_job_is_a_property_list_plutil_accepts"
        "the_derive_launchd_job_is_a_property_list_plutil_accepts"
      ];

      build =
        pkgs: withTests:
        pkgs.rustPlatform.buildRustPackage {
          pname = "bilbo";
          inherit version src;
          cargoLock.lockFile = ./Cargo.lock;
          cargoBuildFlags = [
            "--locked"
            "-p"
            "bilbo-cli"
          ];
          # nixpkgs' cargo hook points zstd and SQLite at system libraries found with pkg-config; unset, the
          # binary builds from the bundled sources Cargo and Bazel build and the tests run on.
          preBuild = ''
            unset ZSTD_SYS_USE_PKG_CONFIG LIBSQLITE3_SYS_USE_PKG_CONFIG
          '';
          doCheck = withTests;
          # The repo's gates run the tests in the debug profile; nixpkgs' hook would run them in release.
          checkType = "debug";
          cargoTestFlags = [
            "--locked"
            "--workspace"
          ];
          checkFlags = map (name: "--skip=${name}") skippedInTheSandbox;
          __darwinAllowLocalNetworking = withTests;
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

      # The fourteen keys of bilbo setup, from apps/cli/src/console/settings.rs and libs/setup/rust/src/keys.rs.
      # A new key joins this list: the activation unsets each one before it sets its own.
      setupVariables = [
        "BILBO_SETUP_AGY_ENABLED"
        "BILBO_SETUP_AGY_PROGRAM"
        "BILBO_SETUP_CLAUDE_ENABLED"
        "BILBO_SETUP_CLAUDE_PROGRAM"
        "BILBO_SETUP_CODEX_ENABLED"
        "BILBO_SETUP_CODEX_PROGRAM"
        "BILBO_SETUP_DERIVE_EVERY"
        "BILBO_SETUP_NOTES"
        "BILBO_SETUP_PLUGIN"
        "BILBO_SETUP_POLL"
        "BILBO_SETUP_REFRESH_EVERY"
        "BILBO_SETUP_TIMER_LOGS"
        "BILBO_SETUP_TIMER_PATH"
        "BILBO_SETUP_WAIT"
      ];

      # The words are joined by this, so the check can read the command as one line.
      joint = " \\\n  ";

      module =
        {
          config,
          lib,
          pkgs,
          ...
        }:
        let
          cfg = config.programs.bilbo;
          option =
            description:
            lib.mkOption {
              type = lib.types.nullOr lib.types.str;
              default = null;
              inherit description;
            };
          program =
            name:
            "The ${name} program, given to bilbo setup as --${name}; null leaves it to setup.${name}.program in the config file, whose default is ${name} looked up on PATH.";
          tool = name: {
            type = lib.types.bool;
            default = true;
            description = "Whether bilbo setup installs the plugin in ${name}.";
          };
          kept = [
            "claude"
            "codex"
            "agy"
          ];
          flag = name: lib.optional (cfg.${name} != null) "--${name}=${lib.escapeShellArg cfg.${name}}";
          off = lib.concatMap (
            name:
            lib.optional (!cfg.tools.${name}) "BILBO_SETUP_${lib.toUpper name}_ENABLED=false"
          ) kept;
          # The tool homes are what the person's shells get from home.sessionVariables, set or unset: the
          # value goes in double quotes as home-manager's own export writes it, so the shell expands it.
          homeValue = name: config.home.sessionVariables.${name} or null;
          home =
            name:
            let
              value = homeValue name;
            in
            lib.optional (value != null) "${name}=\"${
              if builtins.isBool value then lib.boolToString value else toString value
            }\"";
          unsetHome = name: lib.optional (homeValue name == null) "-u ${name}";
          folders = [
            "XDG_CONFIG_HOME=${lib.escapeShellArg config.xdg.configHome}"
            "XDG_DATA_HOME=${lib.escapeShellArg config.xdg.dataHome}"
            "XDG_CACHE_HOME=${lib.escapeShellArg config.xdg.cacheHome}"
            "XDG_STATE_HOME=${lib.escapeShellArg config.xdg.stateHome}"
          ];
          platform =
            if pkgs.stdenv.hostPlatform.isDarwin then
              [ "PATH=${lib.escapeShellArg "${config.home.profileDirectory}/bin:/usr/bin:/bin"}" ]
            else
              [
                "PATH=${lib.escapeShellArg "${config.home.profileDirectory}/bin:${dirOf config.systemd.user.systemctlPath}"}"
                "XDG_RUNTIME_DIR=\"\${XDG_RUNTIME_DIR:-/run/user/$EUID}\""
              ];
          words =
            [ "run" "env" "-u BILBO_HOME" "-u BILBO_CONFIG" ]
            ++ map (name: "-u ${name}") setupVariables
            ++ unsetHome "CODEX_HOME"
            ++ unsetHome "CLAUDE_CONFIG_DIR"
            ++ home "CODEX_HOME"
            ++ home "CLAUDE_CONFIG_DIR"
            ++ off
            ++ folders
            ++ platform
            ++ [
              (lib.concatStringsSep " " (
                [
                  (lib.getExe' cfg.package "bilbo")
                  "setup"
                ]
                ++ flag "claude"
                ++ flag "codex"
                ++ flag "agy"
                ++ flag "notes"
              ))
              "|| warnEcho \"bilbo setup left a step undone: its lines above say which; activate again to try it again, since a bilbo setup run by hand has not this activation's tools, folders and PATH\""
            ];
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
            claude = option (program "claude");
            codex = option (program "codex");
            agy = option (program "agy");
            notes = option "The notes folder bilbo setup makes and follows; null leaves the folder to setup.notes in the config file.";
            tools = lib.genAttrs kept (name: lib.mkOption (tool name));
          };

          config = lib.mkIf cfg.enable {
            home.packages = [ cfg.package ];
            home.activation.bilboSetup = lib.hm.dag.entryAfter [
              "writeBoundary"
              "setupLaunchAgents"
              "installPackages"
            ] (lib.concatStringsSep joint words);
          };
        };

      moduleCheck =
        pkgs:
        let
          hm =
            extra: bilbo:
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
                extra
              ];
            };
          plain = hm { };
          line = c: lib.replaceStrings [ joint ] [ " " ] c.config.home.activation.bilboSetup.data;
          sample = plain {
            enable = true;
            claude = "/opt/claude/bin/claude";
          };
          named = plain {
            enable = true;
            claude = "/opt/claude/bin/claude";
            codex = "/opt/codex/bin/codex";
            agy = "/opt/agy/bin/agy";
            notes = "/data/my notes";
            tools.codex = false;
          };
          dashed = plain {
            enable = true;
            claude = "-claude";
          };
          custom = plain {
            enable = true;
            package = pkgs.runCommand "bilbo-custom" { } "mkdir -p $out/bin; touch $out/bin/bilbo";
          };
          homed = value: hm { home.sessionVariables.CODEX_HOME = value; } { enable = true; };
          spaced = homed "/opt/codex home";
          expanded = homed "$HOME/codex";
          nulled = homed null;
          disabled = plain { enable = false; };
          text = line sample;
          profile = sample.config.home.profileDirectory;
          # Every -u comes before the first word that assigns anything: a word with an = that does not begin with -.
          unsetsFirst =
            c:
            let
              words = lib.splitString " " (line c);
              first = lib.lists.findFirstIndex (w: lib.hasInfix "=" w && !(lib.hasPrefix "-" w)) (lib.length words) words;
            in
            lib.all (i: i < first) (
              lib.filter (i: lib.elemAt words i == "-u") (lib.range 0 (lib.length words - 1))
            );
          cases = [
            {
              name = "the activation holds the program named";
              ok =
                lib.hasInfix "/bin/bilbo setup" text
                && lib.hasInfix "--claude=/opt/claude/bin/claude" text;
            }
            {
              name = "no --codex, --agy or --notes when those are null";
              ok = !(lib.hasInfix "--codex" text || lib.hasInfix "--agy" text || lib.hasInfix "--notes" text);
            }
            {
              name = "the three programs and the notes folder, when named, each as one word";
              ok = lib.hasInfix "--codex=/opt/codex/bin/codex --agy=/opt/agy/bin/agy --notes='/data/my notes'" (line named);
            }
            {
              name = "a program that begins with a dash is one word with its flag";
              ok = lib.hasInfix "--claude=-claude" (line dashed);
            }
            {
              name = "the executable is bin/bilbo, whatever the package is called";
              ok = lib.hasInfix "/bin/bilbo setup" (line custom);
            }
            {
              name = "BILBO_SETUP_CODEX_ENABLED=false when tools.codex is false";
              ok = lib.hasInfix " BILBO_SETUP_CODEX_ENABLED=false " (line named);
            }
            {
              name = "no BILBO_SETUP_<TOOL>_ENABLED= of any value for a tool whose option is true";
              ok =
                !(lib.hasInfix "_ENABLED=" text)
                && !(lib.hasInfix "BILBO_SETUP_CLAUDE_ENABLED=" (line named))
                && !(lib.hasInfix "BILBO_SETUP_AGY_ENABLED=" (line named));
            }
            {
              name = "fourteen BILBO_SETUP_ variables, each unset";
              ok = lib.length setupVariables == 14 && lib.all (name: lib.hasInfix " -u ${name} " text) setupVariables;
            }
            {
              name = "each -u comes before the first assignment of any name";
              ok = lib.all unsetsFirst [
                sample
                named
                spaced
                expanded
                nulled
              ];
            }
            {
              name = "BILBO_HOME and BILBO_CONFIG are unset";
              ok = lib.hasInfix " -u BILBO_HOME -u BILBO_CONFIG " text;
            }
            {
              name = "CODEX_HOME and CLAUDE_CONFIG_DIR are unset when home.sessionVariables sets neither";
              ok = lib.hasInfix " -u CODEX_HOME " text && lib.hasInfix " -u CLAUDE_CONFIG_DIR " text;
            }
            {
              name = "CODEX_HOME is the configuration's value, double-quoted and still one word";
              ok =
                lib.hasInfix " CODEX_HOME=\"/opt/codex home\" " (line spaced)
                && !(lib.hasInfix " -u CODEX_HOME " (line spaced))
                && lib.hasInfix " -u CLAUDE_CONFIG_DIR " (line spaced);
            }
            {
              name = "a home with $HOME is double-quoted, so the shell expands it";
              ok = lib.hasInfix " CODEX_HOME=\"$HOME/codex\" " (line expanded);
            }
            {
              name = "a null CODEX_HOME is unset, never assigned";
              ok = lib.hasInfix " -u CODEX_HOME " (line nulled) && !(lib.hasInfix "CODEX_HOME=" (line nulled));
            }
            {
              name = "the four XDG folders are home-manager's";
              ok =
                lib.hasInfix " XDG_CONFIG_HOME=${sample.config.xdg.configHome} " text
                && lib.hasInfix " XDG_DATA_HOME=${sample.config.xdg.dataHome} " text
                && lib.hasInfix " XDG_CACHE_HOME=${sample.config.xdg.cacheHome} " text
                && lib.hasInfix " XDG_STATE_HOME=${sample.config.xdg.stateHome} " text;
            }
            {
              name = "the PATH opens with the profile's bin";
              ok = lib.hasInfix " PATH=${profile}/bin:" text;
            }
            {
              name = "the activation runs after the packages are installed";
              ok = lib.elem "installPackages" sample.config.home.activation.bilboSetup.after;
            }
            {
              name = "a failed setup is a warning";
              ok = lib.hasInfix "|| warnEcho \"bilbo setup left a step undone: its lines above say which; activate again to try it again, since a bilbo setup run by hand has not this activation's tools, folders and PATH\"" text;
            }
            {
              name = "a disabled module has no activation entry";
              ok = !(disabled.config.home.activation ? bilboSetup);
            }
            {
              name = "the activation package evaluates";
              ok = (builtins.tryEval sample.activationPackage.drvPath).success;
            }
          ];
          failed = lib.filter (c: !c.ok) cases;
        in
        pkgs.runCommand "bilbo-home-manager-module" { } (
          if failed == [ ] then
            "touch $out"
          else
            "echo ${lib.escapeShellArg "the module's check failed: ${(lib.head failed).name}"} >&2; exit 1"
        );
    in
    {
      homeManagerModules.default = module;

      packages = forAll (pkgs: rec {
        bilbo = build pkgs false;
        default = bilbo;
      });

      checks = forAll (pkgs: {
        bilbo = build pkgs false;
        tests = build pkgs true;
        home-manager-module = moduleCheck pkgs;
      });

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
