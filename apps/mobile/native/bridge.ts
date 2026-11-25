export interface AndroidBridge {
  renderNativeUi?(state: string): void;
  getLanguagePreference?(): string;
  setLanguagePreference?(value: string): void;
  getSystemLanguage?(): string;
  scanPairingCode?(): void;
  isAwaitingCertificateTrust?(): boolean;
  takePairingCode?(): string;
  chooseReceiveFolder(): void;
  chooseUploadFiles?(): void;
  getSharedFiles?(): string;
  sendSharedFiles?(revision: string): void;
  removeSharedFile?(revision: string, id: string): void;
  retrySharedFile?(revision: string, id: string): void;
  clearSharedFiles?(revision: string): void;
  getReceiveFolder(): string;
  getMobilePreferences?(): string;
  setMobilePreferences?(value: string): void;
  hasReceiveFolder?(): boolean;
  refreshReceiveFolder?(): void;
  getDeviceName(): string;
  getDeviceModel?(): string;
  downloadFile(path: string, name: string, token: string): void;
  cancelFileTransfer(): void;
  resolveUnfinishedDownload?(id: string, version: string, keep: boolean, request: string): void;
  confirmSensitiveAction(action: string): void;
  changeComputer(): void;
  recoverConnection?(draft: string, request: string): void;
  cancelConnectionRecovery?(request: string): void;
  takeRecoveredDraft?(): string;
  isSystemDark?(): boolean;
  getThemePreference?(): string;
  setThemePreference?(value: string): void;
  setAppearance?(dark: boolean): void;
  setImmersive?(enabled: boolean): void;
  haptic?(): void;
  setKeepAwake?(enabled: boolean): void;
  showPrivacyPolicy?(): void;
  getKeepAlivePreference?(): string;
  setKeepAlivePreference?(enabled: boolean): void;
  setConnectionKeepAlive?(enabled: boolean): void;
  getConnectionKeepAliveState?(): string;
  isControlForegroundReady?(): boolean;
  openBackgroundSettings?(): void;
  getComputerName?(): string;
  getStoredToken?(): string;
  getUsbAccessKey?(): string;
  getInputEpoch?(): number;
  setStoredToken?(value: string): void;
  getStoredDeviceId?(): string;
  setStoredDeviceId?(value: string): void;
  startMicrophone?(id: number): void;
  hasMicrophonePermission?(): boolean;
  stopMicrophone?(id: number): void;
  lockMicrophone?(id: number): void;
  syncSession?(): void;
}

export function androidBridge(): AndroidBridge | undefined {
  return (
    globalThis as typeof globalThis & { PlumeLinkAndroid?: AndroidBridge }
  ).PlumeLinkAndroid;
}

export type MicrophoneState = {
  id: number;
  phase: string;
  level: number;
  message: string;
  locked?: boolean;
  began?: number;
  snapshot?: boolean;
};
