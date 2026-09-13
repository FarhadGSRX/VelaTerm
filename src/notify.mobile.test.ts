import { afterEach, describe, expect, it, vi } from "vitest";
vi.mock("./ipc/transport", () => ({isTauri:false,isRemoteWindow:false}));
import { getNotifyPermission, notify, requestNotifyPermission } from "./notify";

afterEach(() => { Reflect.deleteProperty(window, "__VELATERM_NOTIFICATIONS__"); });
describe("mobile native notification delivery", () => {
  function bridge(permission: string) {
    const native = {getPermission:vi.fn().mockResolvedValue(permission),requestPermission:vi.fn().mockResolvedValue("granted"),send:vi.fn().mockResolvedValue(undefined)};
    Object.defineProperty(window,"__VELATERM_NOTIFICATIONS__",{value:native,configurable:true});
    return native;
  }
  it("uses the native permission and preserves the session and sound in delivery", async () => {
    const native = bridge("granted");
    expect(await getNotifyPermission()).toBe("granted");
    await notify("session", "Fixture", "Complete", true);
    expect(native.send).toHaveBeenCalledWith({sessionId:"session",title:"Fixture",body:"Complete",sound:true});
    expect(native.requestPermission).not.toHaveBeenCalled();
  });
  it("does not prompt again after denial or attempt delivery", async () => {
    const native = bridge("denied");await notify("session","Fixture","Complete");
    expect(native.requestPermission).not.toHaveBeenCalled();expect(native.send).not.toHaveBeenCalled();
  });
  it("requests the OS permission and isolates a failed delivery from the session UI", async () => {
    const native = bridge("default");
    expect(await requestNotifyPermission()).toBe("granted");
    native.getPermission.mockResolvedValue("granted");native.send.mockRejectedValue(new Error("Native delivery failed"));
    await expect(notify("session","Fixture","Complete")).resolves.toBeUndefined();
  });
});
