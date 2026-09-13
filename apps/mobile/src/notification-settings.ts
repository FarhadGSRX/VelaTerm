import { t } from '../../../src/i18n';
import { Remote, type Connection, type PushStatus } from './remote';

const errors: Record<string, Parameters<typeof t>[0]> = {
  PUSH_NOT_CONFIGURED: 'mobile.pushNotConfigured', PUSH_DENIED: 'mobile.pushDenied',
  PUSH_DISABLED: 'mobile.pushDisabled', PUSH_REGISTRATION_FAILED: 'mobile.pushRegistrationFailed',
  PUSH_RELAY_UNAVAILABLE: 'mobile.pushRelayUnavailable', PUSH_HOST_UNAVAILABLE: 'mobile.pushHostUnavailable',
};
export function notificationSettings(records: Connection[]): HTMLElement {
  const panel = document.createElement('section'); panel.className = 'push-settings';
  const hint = document.createElement('p'); hint.textContent = t('mobile.pushHint');
  const state = document.createElement('p'); state.setAttribute('role', 'status'); state.textContent = t('common.loading');
  const controls = document.createElement('div'); controls.className = 'push-controls';
  const detail = document.createElement('p'); detail.className = 'hint'; detail.textContent = t('mobile.pushConnectHint');
  panel.append(hint, state, controls, detail);
  const failure = (error: unknown) => {
    const code = (error as {code?: string})?.code ?? String(error).replace(/^Error: /, '');
    state.textContent = t(errors[code] ?? 'mobile.pushRelayUnavailable');
  };
  const button = (label: Parameters<typeof t>[0], action: () => Promise<unknown>) => {
    const control = document.createElement('button'); control.type = 'button'; control.textContent = t(label);
    control.onclick = async () => {
      controls.querySelectorAll('button').forEach(item => item.disabled = true);
      try { await action() } catch (error) { failure(error) }
      finally { controls.querySelectorAll('button').forEach(item => item.disabled = false) }
    };
    return control;
  };
  const render = (value: PushStatus) => {
    if (!panel.isConnected) return;
    state.textContent = t(value.error ? errors[value.error] ?? 'mobile.pushRelayUnavailable' : !value.configured ? 'mobile.pushNotConfigured' : value.enabled ? 'mobile.pushEnabled' : 'mobile.pushDisabled');
    controls.replaceChildren(button(value.enabled ? 'mobile.pushDisable' : 'mobile.pushEnable', async () => {
      render(await Remote.notifications({action: value.enabled ? 'disable' : 'enable'}));
    }));
    if (value.enabled && value.connections.length) {
      const label = document.createElement('label'); label.textContent = t('mobile.pushTarget');
      const target = document.createElement('select');
      for (const id of value.connections) {
        const option = document.createElement('option'); option.value = id; option.textContent = records.find(row => row.id === id)?.name ?? 'VelaTerm · ' + id.slice(-8); target.append(option);
      }
      label.append(target); controls.append(label, button('mobile.pushTest', async () => {
        await Remote.notifications({action: 'test', id: target.value}); state.textContent = t('mobile.pushTestSent');
      }));
    }
  };
  // Defer until the caller has attached the page to the document.
  queueMicrotask(() => { void Remote.notifications({action:'status'}).then(render).catch(failure) });
  return panel;
}
