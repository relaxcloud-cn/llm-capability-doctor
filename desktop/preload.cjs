const {contextBridge,ipcRenderer}=require('electron');
contextBridge.exposeInMainWorld('agentcheck',{state:()=>ipcRenderer.invoke('agentcheck:state')});
