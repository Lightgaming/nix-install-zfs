{
  description = "NixOS Encrypted ZFS Auto-installer (Rust + Ratatui)";

  nixConfig = {
    extra-substituters = [ "https://saint.cachix.org" ];
    extra-trusted-public-keys = [
      "saint.cachix.org-1:eM94+vbFecwyko6KEWBI6EJrHpPriVb2WJSILAtv3l4="
    ];
  };

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    crane.url = "github:ipetkov/crane";
  };

  outputs = { self, nixpkgs, crane }: let
    system = "x86_64-linux";
    pkgs = nixpkgs.legacyPackages.${system};
    craneLib = crane.mkLib pkgs;

    src = craneLib.cleanCargoSource ./.;

    commonArgs = {
      inherit src;
      strictDeps = true;
      nativeBuildInputs = with pkgs; [ pkg-config ];
      buildInputs = with pkgs; [
        util-linux
        gptfdisk
        dosfstools
        nixos-install-tools
        zfs
        parted
      ];
    };

    cargoArtifacts = craneLib.buildDepsOnly commonArgs;

    install-zfs = craneLib.buildPackage (commonArgs // {
      inherit cargoArtifacts;
      pname = "install-zfs";
      version = "0.1.0";
      cargoExtraArgs = "--bin install-zfs";
      doCheck = false;
    });

    install-zfs-wrapped = pkgs.symlinkJoin {
      name = "install-zfs";
      paths = [ install-zfs ];
      buildInputs = [ pkgs.makeWrapper ];
      postBuild = ''
        wrapProgram $out/bin/install-zfs \
          --prefix PATH : ${pkgs.lib.makeBinPath (with pkgs; [
            util-linux
            gptfdisk
            dosfstools
            nixos-install-tools
            zfs
            parted
            coreutils
            gnugrep
            gnused
          ])}
      '';
    };
  in {
    packages.${system}.default = install-zfs-wrapped;
    apps.${system}.default = {
      type = "app";
      program = "${install-zfs-wrapped}/bin/install-zfs";
    };
  };
}