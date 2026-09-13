import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
vi.mock("../ipc/transport", () => ({invoke:vi.fn()}));
vi.mock("../ipc/info", () => ({copyText:vi.fn()}));
vi.mock("../platform", () => ({platform:{opener:{openExternal:vi.fn().mockResolvedValue(undefined)}}}));
import { invoke } from "../ipc/transport";
import { platform } from "../platform";
import { setLang } from "../i18n";
import { PublicSharingPanel } from "../layout/TitleBar/PublicSharingPanel";
let linked=false;
beforeEach(() => {
  vi.clearAllMocks(); setLang("en"); linked=false;
  vi.mocked(invoke).mockImplementation(async cmd => {
    if(cmd==='public_account_status')return {linked,account:linked?{id:'account',displayName:'Signed-in user'}:undefined} as never;
    if(cmd==='public_remote_options')return {options:{scopes:['project']},projects:[],sessions:[]} as never;
    if(cmd==='public_account_remote_devices')return [] as never;
    if(cmd==='public_account_link')return {url:'https://velaterm.com/account/device?code=fixture',publicKey:'fixture-public-key'} as never;
    if(cmd==='public_account_poll'){linked=true;return {linked:true} as never;}
    if(cmd==='public_account_logout'){linked=false;return undefined as never;}
    throw Error(`Unexpected command: ${cmd}`);
  });
});
afterEach(cleanup);
it('automatically completes approved browser login and displays the account identity', async () => {
  render(<PublicSharingPanel/>);
  await waitFor(()=>expect(invoke).toHaveBeenCalledWith('public_account_status'));
  fireEvent.click(screen.getByRole('button',{name:'Sign in to VelaTerm'}));
  await screen.findByText('Signed-in user');
  expect(platform.opener.openExternal).toHaveBeenCalledWith('https://velaterm.com/account/device?code=fixture');
  expect(invoke).toHaveBeenCalledWith('public_account_poll');
});
it('clears the account and sharing controls after logout succeeds', async () => {
  linked=true;render(<PublicSharingPanel/>);
  await screen.findByText('Signed-in user');
  fireEvent.click(screen.getByRole('button',{name:'Sign out'}));
  await screen.findByRole('button',{name:'Sign in to VelaTerm'});
  expect(invoke).toHaveBeenCalledWith('public_account_logout');
  expect(screen.queryByText('Signed-in user')).toBeNull();
});
it('loads host sharing controls for a signed-in account', async () => {
  linked=true;render(<PublicSharingPanel/>);
  await screen.findByRole('radio',{name:'Project'});
  expect(screen.getByRole('button',{name:'Start sharing'})).toBeTruthy();
  expect(screen.queryByRole('alert')).toBeNull();
});
it('reports a settings load failure and restores the controls after retry', async () => {
  linked=true;
  const original=vi.mocked(invoke).getMockImplementation()!;
  let fail=true;
  vi.mocked(invoke).mockImplementation(async (cmd,args) => {
    if(cmd==='public_remote_options' && fail)throw Error('Unknown command: public_remote_options');
    return original(cmd,args);
  });
  render(<PublicSharingPanel/>);
  expect((await screen.findByRole('alert')).textContent).toContain('Unable to load sharing settings. Please try again.');
  expect(screen.getByText('Signed-in user')).toBeTruthy();
  fail=false;
  fireEvent.click(screen.getByRole('button',{name:'Retry'}));
  await screen.findByRole('radio',{name:'Project'});
  await waitFor(()=>expect(screen.queryByRole('alert')).toBeNull());
});
