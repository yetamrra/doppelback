# doppelback

doppelback is a backup system designed for doing double backups: Individual
systems are first backed up to a local centralized backup host, then that
backup host is backed up to external disks or an offsite host.  This is useful
for (at least) a couple of types of situations:

*  Internal machines that can't reach the internet for a direct offsite backup.
*  Intermittent or slow external internet connectivity.
*  Internal machines that may not always be present (or powered on).

This grew out of my own backup needs as a set of "simple" organically
increasing bash scripts.  At some point, the pile of scripts became
unmaintainable and sort of horrifying.  Rather than clean it up in bash, I've
decided to take this as an opportunity to get some practice in rust.  When the
functionality of this version covers everything that my current backups do,
I'll probably stop.  Feel free to use this for your own backup system if it's
useful, but don't expect a large amount of ongoing maintenance or updates.

doppelback has three main parts:

1.  Backup source(s).  This is a client system to be backed up.  doppelback is
    primarily tested on Fedora and macOS 10.15. Other systems that support ssh
    logins and rsync may work.
2.  A local backup server.  This is a machine with a large disk that will
    periodically pull backups from the sources.  The current design uses btrfs
    snapshots to preserve many months of past backups.
3.  An offsite backup disk.  This is another large btrfs disk that will be
    synchronized by pushing from the local backup server.  The offsite disk can
    be attached to the local backup server, to a different machine, or to a
    remote machine if you have sufficient upload bandwidth to keep up with
    backups.  The offsite disk can be rotated, and the backup server will take
    care of resyncing the outdated disk whenever a swap happens.

doppelback is meant to be mostly self-contained.  The backup source systems
need to have a dedicated backup account.  doppelback should be configured as an
ssh forced command with a passwordless key on this account.  doppelback will
then run itself through sudo when needed; thus, the account only needs
permission to run doppelback with sudo rather than a large list of
hard-to-secure commands.
