# A NixOS service that runs torrss with its feeds and client declared in Nix.
#
# What the store may hold and what it may not is the whole shape of this
# module. The generated file names a feed and the client's address; every
# secret arrives through `extraConfigFiles`, which apply after it.
self:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.torrss;

  # Only the fields that are safe in a world-readable store. `qbit.password`
  # and a feed's `auth` are deliberately absent, and the application's
  # `deny_unknown_fields` rejects anything this does not name.
  configFile = (pkgs.formats.toml { }).generate "torrss.toml" {
    feeds = cfg.feeds;
    qbit = { inherit (cfg.qbit) url username; };
  };
in
{
  options.services.torrss = {
    enable = lib.mkEnableOption "the torrss feed watcher";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.torrss;
      defaultText = lib.literalMD "the `torrss` package of this flake";
      description = "The torrss package to run.";
    };

    host = lib.mkOption {
      type = lib.types.str;
      default = "127.0.0.1";
      description = ''
        Address the listener binds to. Leave it on loopback unless a reverse
        proxy or a firewall stands in front: torrss has no authentication of
        its own.
      '';
    };

    port = lib.mkOption {
      type = lib.types.port;
      default = 3000;
      description = "Port the listener binds to.";
    };

    feeds = lib.mkOption {
      type = lib.types.listOf (
        lib.types.submodule {
          options = {
            name = lib.mkOption {
              type = lib.types.str;
              description = "What the pages call this feed.";
            };

            url = lib.mkOption {
              type = lib.types.str;
              description = ''
                Address of the feed. A passkey in the query is readable by
                anyone on the host, because this reaches the Nix store;
                declare such a feed in `extraConfigFiles` instead.
              '';
            };
          };
        }
      );
      default = [ ];
      description = ''
        Feeds to register. torrss never removes a feed, so dropping one here
        leaves its row and its history in place.
      '';
    };

    qbit.url = lib.mkOption {
      type = lib.types.str;
      default = "http://127.0.0.1:8080";
      description = "Address of the qBittorrent web interface.";
    };

    qbit.username = lib.mkOption {
      type = lib.types.str;
      default = "admin";
      description = "qBittorrent account name.";
    };

    extraConfigFiles = lib.mkOption {
      type = lib.types.listOf lib.types.path;
      default = [ ];
      example = [ "/run/secrets/torrss.toml" ];
      description = ''
        Further configuration files, applied after the generated one, so a
        field set here wins.

        This is where the qBittorrent password and any feed `auth` belong.
        The Nix store is world-readable, so a secret has to come from a file
        a secret manager places on the host.

        Give each path as a string. A Nix path literal copies the file into
        the store, which is what this option exists to avoid.
      '';
    };

    environmentFile = lib.mkOption {
      type = lib.types.nullOr lib.types.path;
      default = null;
      example = "/run/secrets/torrss.env";
      description = ''
        File of environment variables for the service, such as `TORRSS_LOG`.
        Read at start by systemd rather than by Nix, so it stays out of the
        store.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    systemd.services.torrss = {
      description = "torrss feed watcher";
      wantedBy = [ "multi-user.target" ];
      after = [ "network-online.target" ];
      wants = [ "network-online.target" ];

      serviceConfig = {
        ExecStart =
          "${lib.getExe' cfg.package "torrss"}"
          + " --host ${cfg.host}"
          + " --port ${toString cfg.port}"
          + " --db /var/lib/torrss/torrss.db"
          + " --config ${configFile}"
          + lib.concatMapStrings (file: " --config ${file}") cfg.extraConfigFiles;

        # The state directory gives the database a home the dynamic user
        # owns. Without it there is no writable path for a user that exists
        # only while the service runs.
        DynamicUser = true;
        StateDirectory = "torrss";
        Restart = "on-failure";
        EnvironmentFile = lib.mkIf (cfg.environmentFile != null) cfg.environmentFile;
      };
    };
  };
}
