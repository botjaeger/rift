import { describe, expect, it, vi } from "vitest";
import { CAMERA_DENIED, PHOTO_UNREADABLE, pickedPhoto } from "../src/photo";

const PNG = "iVBORw0KGgo=";
const taken = { canceled: false, assets: [{ base64: PNG }] };

describe("a photo from the library or the camera", () => {
  it("returns the photo's base64", async () => {
    expect(await pickedPhoto({ launch: async () => taken })).toEqual({ ok: true, b64: PNG });
  });
  it("a cancel is nothing, not an error", async () => {
    expect(await pickedPhoto({ launch: async () => ({ canceled: true, assets: null }) })).toBeNull();
  });
  it("asks for the camera first and does not open it when refused", async () => {
    const launch = vi.fn(async () => taken);
    expect(await pickedPhoto({ camera: async () => ({ granted: false }), launch })).toEqual({ ok: false, message: CAMERA_DENIED });
    expect(launch).not.toHaveBeenCalled();
  });
  it("a permission request that throws is a refusal", async () => {
    const launch = vi.fn(async () => taken);
    expect(await pickedPhoto({ camera: () => Promise.reject(new Error("no")), launch })).toEqual({ ok: false, message: CAMERA_DENIED });
    expect(launch).not.toHaveBeenCalled();
  });
  it("opens the camera once it is allowed", async () => {
    expect(await pickedPhoto({ camera: async () => ({ granted: true }), launch: async () => taken })).toEqual({ ok: true, b64: PNG });
  });
  it("says so when the picker fails or gives no data, and when the photo is too large", async () => {
    expect(await pickedPhoto({ launch: () => Promise.reject(new Error("x")) })).toEqual({ ok: false, message: PHOTO_UNREADABLE });
    expect(await pickedPhoto({ launch: async () => ({ canceled: false, assets: [{ base64: null }] }) })).toEqual({ ok: false, message: PHOTO_UNREADABLE });
    const big = "A".repeat(15 * 1024 * 1024);
    const r = await pickedPhoto({ launch: async () => ({ canceled: false, assets: [{ base64: big }] }) });
    expect(r).toMatchObject({ ok: false });
    expect((r as { message: string }).message).toContain("larger than");
  });
});
