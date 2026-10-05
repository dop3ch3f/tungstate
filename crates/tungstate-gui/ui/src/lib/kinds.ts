// The kinds of place a connection can be, in the order a person choosing one
// should see them, with the words each is described by. The engine's
// `Scheme::ALL` is the same six.

export type Kind = "fs" | "smb" | "webdav" | "ftps" | "ftp" | "s3";

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
  { id: "webdav", label: "Web folders", tag: "WebDAV", line: "Files over the web. Most NAS boxes offer it, as do Nextcloud and many hosting services." },
  { id: "fs", label: "A folder this Mac can reach", tag: "Folder", line: "A drive or a share that is already mounted, reached like any folder." },
  { id: "s3", label: "S3 storage", tag: "S3", line: "A bucket on AWS, Backblaze B2, Cloudflare R2, MinIO, or a NAS's own object store." },
  { id: "ftps", label: "FTP over TLS", tag: "FTPS", line: "FTP with the password and the files encrypted." },
  { id: "ftp", label: "FTP", tag: "FTP", line: "Plain FTP. The password and the files cross the network unencrypted." },
];

export const kindOf = (scheme: string): KindInfo =>
  KINDS.find((k) => k.id === scheme) ?? { id: "fs", label: scheme, tag: scheme.toUpperCase(), line: "" };

/** Where an S3 bucket can live, and what its endpoint looks like there. AWS
 *  needs none: the region says where. */
export const S3_SERVICES = [
  { id: "aws", label: "AWS", endpoint: "", example: "", region: "us-east-1" },
  { id: "b2", label: "Backblaze B2", endpoint: "", example: "https://s3.us-west-004.backblazeb2.com", region: "" },
  { id: "r2", label: "Cloudflare R2", endpoint: "", example: "https://<account>.r2.cloudflarestorage.com", region: "auto" },
  { id: "other", label: "MinIO, a NAS, or another", endpoint: "", example: "http://nas.local:9000", region: "" },
] as const;

export type S3Service = (typeof S3_SERVICES)[number]["id"];

/** Which service an endpoint belongs to, for editing a saved connection. */
export function serviceOf(endpoint: string | undefined): S3Service {
  if (!endpoint) return "aws";
  if (endpoint.includes("backblazeb2.com")) return "b2";
  if (endpoint.includes("r2.cloudflarestorage.com")) return "r2";
  return "other";
}

/** An SMB root split into the share and the folder inside it. Either slash,
 *  as the engine reads it. */
export function splitShare(root: string): { share: string; inside: string } {
  const parts = root.split(/[/\\]/).map((p) => p.trim()).filter(Boolean);
  return { share: parts[0] ?? "", inside: parts.slice(1).join("/") };
}
