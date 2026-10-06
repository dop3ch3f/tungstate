// The kinds of place a connection can be, in the order a person choosing one
// should see them, with the words each is described by. The engine's
// `Scheme::ALL` is the same seven.

export type Kind = "fs" | "smb" | "sftp" | "webdav" | "ftps" | "ftp" | "s3";

export interface KindInfo {
  id: Kind;
  /** What it is called in a list. */
  label: string;
  /** One line on when to choose it. */
  line: string;
  /** The short name on a connection's row. */
  tag: string;
}

export const KINDS: KindInfo[] = [
  { id: "smb", label: "A shared folder", tag: "SMB", line: "Windows file sharing. Most NAS boxes and other computers offer it, and nothing needs mounting." },
  { id: "sftp", label: "Files over SSH", tag: "SFTP", line: "Encrypted, and on almost every NAS. Signs in with a password or an SSH key." },
  { id: "webdav", label: "Web folders", tag: "WebDAV", line: "Files over the web: Nextcloud, pCloud, Koofr, Yandex Disk, and most NAS boxes." },
  { id: "fs", label: "A folder this Mac can reach", tag: "Folder", line: "A drive or a share that is already mounted, reached like any folder." },
  { id: "s3", label: "S3 storage", tag: "S3", line: "A bucket on Backblaze B2, Cloudflare R2, Wasabi, AWS and the like, with an access key." },
  { id: "ftps", label: "FTP over TLS", tag: "FTPS", line: "FTP with the password and the files encrypted." },
  { id: "ftp", label: "FTP", tag: "FTP", line: "Plain FTP. The password and the files cross the network unencrypted." },
];

export const kindOf = (scheme: string): KindInfo =>
  KINDS.find((k) => k.id === scheme) ?? { id: "fs", label: scheme, tag: scheme.toUpperCase(), line: "" };

/** Where an S3 bucket can live, what its endpoint looks like there, and
 *  where its keys are made. AWS needs no endpoint: the region says where.
 *  An empty region is asked of the service when the connection opens. */
export const S3_SERVICES = [
  { id: "aws", label: "AWS", endpoint: "", example: "", region: "us-east-1",
    keys: "In AWS: IAM, then Users, Security credentials, Create access key." },
  { id: "b2", label: "Backblaze B2", endpoint: "", example: "https://s3.us-west-004.backblazeb2.com", region: "",
    keys: "In Backblaze: Application Keys, then Add a New Application Key. The master key does not work here." },
  { id: "r2", label: "Cloudflare R2", endpoint: "", example: "https://<account>.r2.cloudflarestorage.com", region: "auto",
    keys: "In Cloudflare: R2, then Manage API tokens. The secret is shown once." },
  { id: "wasabi", label: "Wasabi", endpoint: "", example: "https://s3.eu-central-1.wasabisys.com", region: "",
    keys: "In the Wasabi console: Access Keys. Use the endpoint for the bucket's own region." },
  { id: "idrive", label: "iDrive e2", endpoint: "", example: "copy it from Enabled regions in your e2 dashboard", region: "",
    keys: "In iDrive e2: Access Keys, then Create Access Key for the bucket's region." },
  { id: "storj", label: "Storj", endpoint: "", example: "https://gateway.storjshare.io", region: "",
    keys: "In Storj: Access Keys, then New Access Key, as S3 credentials." },
  { id: "spaces", label: "DigitalOcean Spaces", endpoint: "", example: "https://nyc3.digitaloceanspaces.com", region: "us-east-1",
    keys: "In DigitalOcean: Spaces Object Storage, then Access Keys." },
  { id: "hetzner", label: "Hetzner Object Storage", endpoint: "", example: "https://fsn1.your-objectstorage.com", region: "",
    keys: "In the Hetzner Console: your project, then S3 credentials. The secret is shown once." },
  { id: "c2", label: "Synology C2", endpoint: "", example: "https://us-001.s3.synologyc2.net", region: "",
    keys: "In C2 Object Storage: Access Keys." },
  { id: "mega", label: "MEGA S4", endpoint: "", example: "https://s3.eu-amsterdam.megas4.com", region: "",
    keys: "In MEGA: Object storage, then Keys, Create key." },
  { id: "scaleway", label: "Scaleway", endpoint: "", example: "https://s3.fr-par.scw.cloud", region: "",
    keys: "In Scaleway: IAM, then API keys, for the bucket's project." },
  { id: "other", label: "MinIO, a NAS, or another", endpoint: "", example: "http://nas.local:9000", region: "",
    keys: "Wherever the service makes access keys." },
] as const;

export type S3Service = (typeof S3_SERVICES)[number]["id"];

/** Which service an endpoint belongs to, for editing a saved connection. */
export function serviceOf(endpoint: string | undefined): S3Service {
  if (!endpoint) return "aws";
  const by: [string, S3Service][] = [
    ["backblazeb2.com", "b2"], ["r2.cloudflarestorage.com", "r2"], ["wasabisys.com", "wasabi"],
    ["idrivee2", "idrive"], ["storjshare.io", "storj"], ["digitaloceanspaces.com", "spaces"],
    ["your-objectstorage.com", "hetzner"], ["synologyc2.net", "c2"], ["megas4.com", "mega"],
    ["scw.cloud", "scaleway"],
  ];
  return by.find(([host]) => endpoint.includes(host))?.[1] ?? "other";
}

/** Services reached over WebDAV, with the address each uses. A `template`
 *  has a part the person fills in; a fixed address is filled for them. */
export const WEBDAV_SERVICES = [
  { id: "nas", label: "A NAS (Synology, QNAP…)", address: "", template: "https://nas.local:5006",
    note: "On a Synology it is usually https://its-name:5006, once WebDAV Server is installed." },
  { id: "nextcloud", label: "Nextcloud", address: "", template: "https://your-server/remote.php/dav/files/USERNAME",
    note: "With two-step sign-in on, make an app password in Settings, then Security." },
  { id: "owncloud", label: "ownCloud", address: "", template: "https://your-server/remote.php/dav/files/USERNAME",
    note: "With two-step sign-in on, make an app password." },
  { id: "pcloud", label: "pCloud (US)", address: "https://webdav.pcloud.com", template: "",
    note: "Paid plans only, and WebDAV stops working when two-step sign-in is on." },
  { id: "pcloud-eu", label: "pCloud (EU)", address: "https://ewebdav.pcloud.com", template: "",
    note: "Paid plans only, and WebDAV stops working when two-step sign-in is on." },
  { id: "koofr", label: "Koofr", address: "https://app.koofr.net/dav/Koofr", template: "",
    note: "Use an app password from Koofr's Preferences, then Password." },
  { id: "yandex", label: "Yandex Disk", address: "https://webdav.yandex.ru", template: "",
    note: "Make an app password for WebDAV in Yandex ID, then Security." },
  { id: "storagebox", label: "Hetzner Storage Box", address: "", template: "https://u123456.your-storagebox.de",
    note: "Turn on WebDAV and external reachability for the Storage Box in the Hetzner Console." },
  { id: "kdrive", label: "Infomaniak kDrive", address: "", template: "https://123456.connect.kdrive.infomaniak.com",
    note: "Use your kDrive's number. With two-step sign-in on, make an app password." },
  { id: "seafile", label: "Seafile", address: "", template: "https://your-server/seafdav",
    note: "With two-step sign-in on, Seafile may give you a separate WebDAV password." },
  { id: "4shared", label: "4shared", address: "https://webdav.4shared.com", template: "", note: "" },
  { id: "opendrive", label: "OpenDrive", address: "https://webdav.opendrive.com", template: "", note: "" },
  { id: "other", label: "Another server", address: "", template: "https://server/path", note: "" },
] as const;

export type WebDavService = (typeof WEBDAV_SERVICES)[number]["id"];

/** Which WebDAV service an address belongs to, for editing a saved one. */
export function webdavServiceOf(endpoint: string | undefined): WebDavService {
  if (!endpoint) return "nas";
  const fixed = WEBDAV_SERVICES.find((s) => s.address && endpoint.startsWith(s.address));
  if (fixed) return fixed.id;
  const by: [string, WebDavService][] = [
    ["your-storagebox.de", "storagebox"], ["kdrive.infomaniak.com", "kdrive"], ["/seafdav", "seafile"],
    ["/remote.php/dav", "nextcloud"],
  ];
  return by.find(([part]) => endpoint.includes(part))?.[1] ?? "other";
}

/** Services planned but not built yet: signing in through the browser. */
export const SOON = [
  { label: "Google Drive", line: "Sign in with your Google account. Until then, the Google Drive app's folder is under Cloud drives." },
  { label: "OneDrive", line: "Sign in with your Microsoft account. Until then, the OneDrive app's folder is under Cloud drives." },
  { label: "Dropbox", line: "Sign in with your Dropbox account. Until then, the Dropbox app's folder is under Cloud drives." },
] as const;

/** An SMB root split into the share and the folder inside it. Either slash,
 *  as the engine reads it. */
export function splitShare(root: string): { share: string; inside: string } {
  const parts = root.split(/[/\\]/).map((p) => p.trim()).filter(Boolean);
  return { share: parts[0] ?? "", inside: parts.slice(1).join("/") };
}
