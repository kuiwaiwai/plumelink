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
  getReceiveFolder(): string;
  hasReceiveFolder?(): boolean;
  getDeviceName(): string;
  downloadFile(path: string, name: string, token: string): void;
  cancelFileTransfer(): void;
  confirmSensitiveAction(action: string): void;
  changeComputer(): void;
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
  getComputerName?(): string;
  getStoredToken?(): string;
  setStoredToken?(value: string): void;
  getStoredDeviceId?(): string;
  setStoredDeviceId?(value: string): void;
  startMicrophone?(id: number): void;
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
