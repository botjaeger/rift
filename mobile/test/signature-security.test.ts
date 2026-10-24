import { constants, generateKeyPairSync, privateEncrypt } from "node:crypto";
import { createRequire } from "node:module";
import { describe, expect, it } from "vitest";

const require = createRequire(import.meta.url);
const updates = createRequire(require.resolve("expo-updates/package.json"));
const certificates = createRequire(updates.resolve("@expo/code-signing-certificates/package.json"));
const forge = certificates("node-forge") as {
  pki: { publicKeyFromPem(pem: string): { verify(digest: string, signature: string): boolean } };
  md: { sha256: { create(): { update(text: string): { digest(): { getBytes(): string } } } } };
  asn1: {
    create(tag: number, type: number, constructed: boolean, value: unknown): unknown;
    oidToDer(oid: string): { getBytes(): string };
    toDer(object: unknown): { getBytes(): string };
  };
};

describe("Expo signing dependency security", () => {
  it("accepts valid RSA DigestInfo and rejects ignored nested elements", () => {
    const keys = generateKeyPairSync("rsa", { modulusLength: 1024 });
    const publicKey = forge.pki.publicKeyFromPem(keys.publicKey.export({ format: "pem", type: "pkcs1" }).toString());
    const digest = forge.md.sha256.create().update("disposable signature fixture").digest().getBytes();
    const node = (type: number, constructed: boolean, value: unknown) => forge.asn1.create(0, type, constructed, value);
    const oid = node(6, false, forge.asn1.oidToDer("2.16.840.1.101.3.4.2.1").getBytes());
    const nil = node(5, false, "");
    const garbage = node(4, false, "ignored attacker padding");
    const sign = (algorithm: unknown[]) => {
      const info = node(16, true, [node(16, true, algorithm), node(4, false, digest)]);
      const der = forge.asn1.toDer(info).getBytes();
      return privateEncrypt({ key: keys.privateKey, padding: constants.RSA_PKCS1_PADDING }, Buffer.from(der, "binary")).toString("binary");
    };
    for (const valid of [[oid], [oid, nil]]) expect(publicKey.verify(digest, sign(valid))).toBe(true);
    for (const invalid of [[oid, garbage], [oid, nil, garbage]]) {
      expect(() => publicKey.verify(digest, sign(invalid))).toThrow(/DigestInfo/);
    }
  });
});
