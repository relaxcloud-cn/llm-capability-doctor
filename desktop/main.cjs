const {app,BrowserWindow,ipcMain,dialog}=require('electron');
const path=require('node:path');
let window;
if(process.env.AGENTCHECK_SOFTWARE_RENDERING==='1')app.disableHardwareAcceleration();
app.whenReady().then(()=>{
 window=new BrowserWindow({width:1344,height:1000,minWidth:800,minHeight:600,title:'AgentCheck',show:false,backgroundColor:'#ffffff',webPreferences:{preload:path.join(__dirname,'preload.cjs'),contextIsolation:true,nodeIntegration:false,sandbox:true}});
 window.webContents.setWindowOpenHandler(()=>({action:'deny'}));
 window.webContents.on('will-navigate',event=>event.preventDefault());
 window.loadFile(path.join(__dirname,'index.html'));
 window.once('ready-to-show',()=>{window.show();process.stdout.write(JSON.stringify({type:'ready'})+'\n');});
});
app.on('window-all-closed',()=>app.quit());
ipcMain.handle('agentcheck:state',()=>({status:'offline-smoke',history:[]}));
