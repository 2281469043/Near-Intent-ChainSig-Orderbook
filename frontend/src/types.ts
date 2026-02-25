export interface Intent {
  id: number;
  maker: string;
  src_asset: string;
  src_amount: string; // u128 stringified
  filled_amount: string;
  dst_asset: string;
  dst_amount: string;
  status: string | { [key: string]: unknown };
  expires_at: number;
  dst_address: string;
  src_path: string;
}

export function statusLabel(s: Intent["status"]): string {
  if (typeof s === "string") return s;
  return Object.keys(s)[0];
}
