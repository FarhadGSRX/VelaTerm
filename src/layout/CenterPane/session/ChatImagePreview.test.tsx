import { cleanup,fireEvent,render,screen,waitFor } from "@testing-library/react";
import { afterEach,expect,it,vi } from "vitest";
import { ChatImagePreview } from "./ChatImagePreview";

vi.mock("../../../platform/imageActions",()=>({copyImage:vi.fn(async()=>{}),saveImage:vi.fn(async()=>{})}));
import { saveImage } from "../../../platform/imageActions";
afterEach(()=>{cleanup();vi.clearAllMocks();});

it("loads the verified original for viewing and uses original bytes for image actions",async()=>{
  const original=vi.fn(async()=>"data:image/png;base64,original-bytes");
  const loadOriginal=vi.fn(async()=>"data:image/png;base64,view-original-bytes");
  render(<ChatImagePreview src="data:image/png;base64,thumbnail-bytes" alt="Saved image" onClose={()=>{}} original={original} loadOriginal={loadOriginal}/>);
  await waitFor(()=>expect(screen.getByRole("img").getAttribute("src")).toBe("data:image/png;base64,view-original-bytes"));
  expect(loadOriginal).toHaveBeenCalledTimes(1);expect(original).not.toHaveBeenCalled();
  fireEvent.contextMenu(screen.getByRole("img"));
  fireEvent.click(screen.getByText("Save image"));
  await waitFor(()=>expect(saveImage).toHaveBeenCalledWith("data:image/png;base64,original-bytes"));
  expect(original).toHaveBeenCalledTimes(1);
});

it("reports a rejected original view without substituting the thumbnail",async()=>{
  render(<ChatImagePreview src="data:image/png;base64,thumbnail-bytes" alt="Saved image" onClose={()=>{}} loadOriginal={async()=>{throw new Error("Original image requires download");}}/>);
  await waitFor(()=>expect(screen.getByRole("alert").textContent).toContain("Original image requires download"));
  expect(screen.queryByRole("img")).toBeNull();
});
