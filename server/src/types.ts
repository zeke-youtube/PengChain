export type Token = {
  id: string;
  name: string;
  symbol: string;
  creatorWalletId: string | null;
  maxSupply: number | null;
  issuedSupply: number;
  mineable: boolean;
  difficulty: number;
  createdAt: number;
};
export type Session = { id: string; walletId: string; expiresAt: number };
