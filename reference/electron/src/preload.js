'use strict';
// Arayüze yalnızca bu dar API açılır. Arayüz Node.js'e veya dosya sistemine doğrudan erişemez.
const { contextBridge, ipcRenderer, webUtils } = require('electron');

contextBridge.exposeInMainWorld('kopru', {
  state: () => ipcRenderer.invoke('state'),
  onState: (fn) => ipcRenderer.on('state', (_e, s) => fn(s)),
  connect: (id) => ipcRenderer.invoke('connect', id),
  connectAddress: (addr) => ipcRenderer.invoke('connectAddress', addr),
  disconnect: () => ipcRenderer.invoke('disconnect'),
  confirmPairing: (ok) => ipcRenderer.invoke('confirmPairing', ok),
  forget: (id) => ipcRenderer.invoke('forget', id),
  settings: (patch) => ipcRenderer.invoke('settings', patch),
  chooseDownloadDir: () => ipcRenderer.invoke('chooseDownloadDir'),
  pickFiles: () => ipcRenderer.invoke('pickFiles'),
  sendDropped: (fileList) => ipcRenderer.invoke('sendPaths', [...fileList].map((f) => webUtils.getPathForFile(f)).filter(Boolean)),
  fileAccept: (id) => ipcRenderer.invoke('fileAccept', id),
  fileReject: (id) => ipcRenderer.invoke('fileReject', id),
  fileCancel: (id) => ipcRenderer.invoke('fileCancel', id),
  fileShow: (id) => ipcRenderer.invoke('fileShow', id),
  openDownloads: () => ipcRenderer.invoke('openDownloads'),
});
