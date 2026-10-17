export type ProductId = 'runjam' | 'musicplus' | 'pasteguana';
export type ProductAccount = {
  user: { id: string; email: string }; tier: 'free' | 'pro';
  wallet: { balanceMicros: number; availableMicros: number; reservedMicros: number; rechargedMicros: number; spentMicros: number };
};
export class PlatformClient {
  constructor(product: ProductId, options?: { fetch?: typeof fetch });
  product: ProductId;
  setToken(token: string): void;
  setTelemetryEnabled(value: boolean): void;
  sendCode(email: string): Promise<{ sent: boolean }>;
  login(email: string, code: string): Promise<{ token: string; account: ProductAccount }>;
  logout(): Promise<{ ok: boolean }>;
  catalog(): Promise<Record<string, unknown>>;
  account(): Promise<ProductAccount>;
  ledger(): Promise<Record<string, unknown>>;
  checkout(sku: string, machineId?: string): Promise<{ checkoutUrl: string; orderId: string }>;
  restore(machineId: string): Promise<Record<string, unknown>>;
  redeem(code: string, machineId: string): Promise<{ licenseCode: string }>;
  registerInstallation(body: Record<string, unknown>): Promise<Record<string, unknown>>;
  events(body: Record<string, unknown>): Promise<Record<string, unknown>>;
  errors(body: Record<string, unknown>): Promise<Record<string, unknown>>;
  feedback(body: Record<string, unknown>): Promise<Record<string, unknown>>;
}
