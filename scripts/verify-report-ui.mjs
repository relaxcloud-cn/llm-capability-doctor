import fs from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {pathToFileURL} from 'node:url';

const receipt=JSON.parse(await fs.readFile(process.argv[2],'utf8'));
const browser=process.argv[3]||process.env.AGENTCHECK_CHROME||(process.platform==='darwin'?'/Applications/Google Chrome.app/Contents/MacOS/Google Chrome':'/usr/bin/chromium');
const output=path.join(receipt.output,'visual');await fs.mkdir(output,{recursive:true});
const profile=await fs.mkdtemp(path.join(os.tmpdir(),'agentcheck-report-ui-'));
const pause=ms=>new Promise(resolve=>setTimeout(resolve,ms));
const child=spawn(browser,['--headless=new','--remote-debugging-port=0',`--user-data-dir=${profile}`,'--no-first-run','--no-default-browser-check','--disable-background-networking','--disable-component-update','--hide-scrollbars','about:blank'],{stdio:['ignore','ignore','pipe']});
let diagnostics='';child.stderr.on('data',data=>diagnostics+=data);child.on('error',error=>diagnostics+=error.message);
let socket;
try{
 let info;
 for(let n=0;n<200;n++){try{info=(await fs.readFile(path.join(profile,'DevToolsActivePort'),'utf8')).trim().split('\n');break;}catch{await pause(100);}}
 assert(info,diagnostics);
 socket=new WebSocket(`ws://127.0.0.1:${info[0]}${info[1]}`);
 await new Promise((resolve,reject)=>{socket.onopen=resolve;socket.onerror=reject;});
 let serial=0;const pending=new Map();const exceptions=[];
 socket.onmessage=event=>{const m=JSON.parse(event.data);if(m.id){const handler=pending.get(m.id);pending.delete(m.id);m.error?handler.reject(new Error(JSON.stringify(m.error))):handler.resolve(m.result);}else if(m.method==='Runtime.exceptionThrown')exceptions.push(m.params);};
 const call=(method,params={},sessionId)=>new Promise((resolve,reject)=>{const id=++serial;pending.set(id,{resolve,reject});socket.send(JSON.stringify({id,method,params,...(sessionId?{sessionId}:{})}));});
 async function tab(file){
  const {targetId}=await call('Target.createTarget',{url:'about:blank'});
  const {sessionId}=await call('Target.attachToTarget',{targetId,flatten:true});
  const send=(method,params={})=>call(method,params,sessionId);
  const evaluate=async expression=>{const result=await send('Runtime.evaluate',{expression,returnByValue:true,awaitPromise:true});assert(!result.exceptionDetails,JSON.stringify(result.exceptionDetails));return result.result.value;};
  await send('Page.enable');await send('Runtime.enable');await send('Network.enable');
  await send('Network.setBlockedURLs',{urls:['http://*','https://*']});
  await send('Emulation.setEmulatedMedia',{features:[{name:'prefers-reduced-motion',value:'reduce'}]});
  const viewport=async width=>{await send('Emulation.setDeviceMetricsOverride',{width,height:960,deviceScaleFactor:1,mobile:width<650});await pause(100);};
  await viewport(1440);await send('Page.navigate',{url:pathToFileURL(file).href});
  for(let n=0;n<100;n++){if(await evaluate('document.readyState==="complete"'))break;await pause(100);}
  await evaluate('document.fonts.ready.then(()=>true)');
  const screenshot=async(name,selector=null)=>{await evaluate(selector?`document.querySelector(${JSON.stringify(selector)}).scrollIntoView({block:'start',behavior:'instant'})`:'window.scrollTo(0,0)');await pause(120);const image=await send('Page.captureScreenshot',{format:'png',captureBeyondViewport:false});if(name)await fs.writeFile(path.join(output,name+'.png'),Buffer.from(image.data,'base64'));return image.data;};
  return {targetId,send,evaluate,viewport,screenshot};
 }
 const live=await tab(receipt.live_report);
 const initial=await live.evaluate(`({headings:[...document.querySelectorAll('main>section h2')].map(e=>e.innerText),rows:[...document.querySelectorAll('#check-rows tr')].map(e=>({id:e.dataset.check,status:e.dataset.status,advice:e.dataset.advice})),text:document.body.innerText,open:document.getElementById('record-appendix').open})`);
 assert.deepEqual(initial.headings,['检测结论','检测结果与基线对照','使用建议']);
 assert.equal(initial.rows.length,8);assert(initial.rows.every(r=>r.status==='passed'));assert(!initial.open);
 for(const absent of ['设计预览','虚构','示例','OpenAI','gpt-6-astra','94%'])assert(!initial.text.includes(absent),absent);
 assert(initial.text.includes('4.32 秒'));assert(initial.text.includes('256K'));assert(initial.text.includes('64 并发'));
 assert.equal(await live.evaluate('document.querySelectorAll("main>section table").length'),1);
 const filters=[];
 for(const [filter,count] of [['advice',1],['failed',0],['all',8]]){
  await live.evaluate(`document.querySelector('[data-filter="${filter}"]').click()`);
  const visible=await live.evaluate('document.querySelectorAll("#check-rows tr:not([hidden])").length');assert.equal(visible,count);
  assert.equal(await live.evaluate('!document.getElementById("filter-empty").hidden'),count===0);filters.push({filter,visible});
 }
 await live.screenshot('live-desktop-top');await live.screenshot('live-desktop-results','#checks');await live.screenshot('live-desktop-guidance','#guidance');
 const dynamic=await live.evaluate(`({pageTitle:document.querySelector('.page-title').innerHTML,tableModel:document.querySelector('thead .customer-col .col-note').innerHTML,hero:document.querySelector('.hero').innerHTML,rows:[...document.querySelectorAll('#check-rows tr')].map(e=>({title:e.querySelector('th').innerHTML,cell:e.querySelector('.customer-col').innerHTML})),guidanceHead:document.querySelector('#guidance .section-head').innerHTML,guidance:[...document.querySelectorAll('#guidance .advice-card')].map(e=>e.innerHTML),footer:document.querySelector('.footer').innerHTML,nojs:document.querySelector('.no-js-message').innerHTML,printScope:document.querySelector('.print-scope').innerHTML})`);
 const reference=await tab(receipt.reference);
 await reference.evaluate(`(() => {
  const data=${JSON.stringify(dynamic).replace(/</g,'\\u003c')};
  document.querySelector('.page-title').innerHTML=data.pageTitle;
  document.querySelector('.preview-notice').remove();
  document.querySelector('.view-label').remove();
  document.querySelector('.benchmark-notice').remove();
  document.querySelectorAll('.reference-col').forEach(e=>e.remove());
  document.querySelector('thead .customer-col .col-note').innerHTML=data.tableModel;
  document.querySelector('.hero').innerHTML=data.hero;
  document.querySelectorAll('#check-rows tr').forEach((row,i)=>{row.querySelector('th').innerHTML=data.rows[i].title;row.querySelector('.customer-col').innerHTML=data.rows[i].cell;});
  document.querySelector('#guidance .section-head').innerHTML=data.guidanceHead;
  document.querySelectorAll('#guidance .advice-card').forEach((e,i)=>e.innerHTML=data.guidance[i]);
  document.querySelector('.footer').innerHTML=data.footer;
  document.querySelector('.no-js-message').innerHTML=data.nojs;
  document.querySelector('.print-scope').innerHTML=data.printScope;
 })()`);
 const parity=[];
 const regions=[['top',null],['results','#checks'],['guidance','#guidance']];
 for(const width of [1440,390]){
  await live.viewport(width);await reference.viewport(width);
  for(const [name,selector] of regions){
   const a=await live.screenshot(`actual-${width}-${name}`,selector);
   const b=await reference.screenshot(`reference-${width}-${name}`,selector);
   const diff=await live.evaluate(`(async()=>{const load=src=>new Promise((resolve,reject)=>{const image=new Image();image.onload=()=>resolve(image);image.onerror=reject;image.src=src;});const a=await load('data:image/png;base64,${a}'),b=await load('data:image/png;base64,${b}');if(a.width!==b.width||a.height!==b.height)return {dimensions:false};const canvas=document.createElement('canvas');canvas.width=a.width;canvas.height=a.height;const ctx=canvas.getContext('2d',{willReadFrequently:true});ctx.drawImage(a,0,0);const x=ctx.getImageData(0,0,a.width,a.height).data;ctx.clearRect(0,0,a.width,a.height);ctx.drawImage(b,0,0);const y=ctx.getImageData(0,0,b.width,b.height).data;let different=0;for(let i=0;i<x.length;i+=4){if(x[i]!==y[i]||x[i+1]!==y[i+1]||x[i+2]!==y[i+2]||x[i+3]!==y[i+3])different++;}return {dimensions:true,different,total:a.width*a.height};})()`);
   parity.push({width,region:name,...diff});
  }
 }
 await live.evaluate("document.querySelector('[data-open=proof-tools]').click()");
 assert(await live.evaluate('document.getElementById("record-appendix").open && document.getElementById("proof-tools").open'));
 await live.evaluate("document.querySelectorAll('details').forEach(e=>e.open=true)");
 const widths=[];
 for(const width of [1440,768,390,320]){await live.viewport(width);const value=await live.evaluate('({viewport:innerWidth,width:document.documentElement.scrollWidth,height:document.documentElement.scrollHeight})');assert(value.width<=value.viewport,JSON.stringify(value));widths.push(value);}
 await live.viewport(1440);await live.evaluate("document.querySelector('[data-filter=failed]').click()");
 const pdf=await live.send('Page.printToPDF',{printBackground:true,preferCSSPageSize:true});const bytes=Buffer.from(pdf.data,'base64');const pages=(bytes.toString('latin1').match(/\/Type \/Page\b/g)||[]).length;assert(pages<=3);await fs.writeFile(path.join(output,'live-report.pdf'),bytes);
 assert.equal(await live.evaluate('document.querySelectorAll("#check-rows tr:not([hidden])").length'),0);
 await live.evaluate("document.querySelector('[data-filter=all]').click()");
 const downloads=path.join(output,'downloads-'+Date.now());await fs.mkdir(downloads,{recursive:true});
 await call('Browser.setDownloadBehavior',{behavior:'allow',downloadPath:downloads,eventsEnabled:true});
 await live.evaluate("document.getElementById('download-records').click()");await pause(500);
 const files=await fs.readdir(downloads);assert.equal(files.length,1);const downloaded=await fs.readFile(path.join(downloads,files[0]));const actualZip=await fs.readFile(path.join(path.dirname(receipt.live_report),'records.zip'));assert(downloaded.equals(actualZip));
 const fixtures=[];
 for(const sample of receipt.fixtures){
  const page=await tab(sample.report);const state=await page.evaluate('({rows:[...document.querySelectorAll("#check-rows tr")].map(e=>e.dataset.status),text:document.body.innerText,injected:!!window.injected})');assert(!state.injected);
  if(sample.case==='mixed'){assert.equal(state.rows.filter(s=>s==='passed').length,5);assert(state.text.includes('暂不建议接入平台'));await page.evaluate("document.querySelector('[data-filter=unfinished]').click()");assert.equal(await page.evaluate('document.querySelectorAll("#check-rows tr:not([hidden])").length'),2);await page.evaluate("document.querySelector('[data-filter=all]').click()");}
  if(sample.case==='incomplete'){assert(state.text.includes('提前停止'));assert.equal(state.rows.filter(s=>s==='not_executed').length,7);}
  if(sample.case==='empty'){assert.equal(state.rows.length,0);assert(state.text.includes('本次没有检测项目'));}
  if(sample.case==='legacy'){assert(state.text.includes('检测版本 未记录'));}
  await page.screenshot('state-'+sample.case);await page.viewport(390);const dimension=await page.evaluate('({width:document.documentElement.scrollWidth,viewport:innerWidth})');assert(dimension.width<=dimension.viewport);await page.screenshot('state-'+sample.case+'-mobile');
  fixtures.push({case:sample.case,statuses:state.rows});await call('Target.closeTarget',{targetId:page.targetId});
 }
 await live.send('Emulation.setScriptExecutionDisabled',{value:true});await live.send('Page.navigate',{url:'about:blank'});await live.send('Page.navigate',{url:pathToFileURL(receipt.live_report).href+'?no-script=1'});await pause(400);
 assert.equal(await live.evaluate('document.querySelectorAll("#check-rows tr").length'),8);assert.equal(await live.evaluate('document.querySelectorAll("main>section").length'),3);assert.equal(await live.evaluate('document.getElementById("record-appendix").open'),false);await live.screenshot('live-no-script');
 assert.equal(exceptions.length,0);
 const result={referenceNormalization:'Only actual data text/metric markup and production-only removal of preview controls were applied to the approved reference; its CSS and page layout were not changed.',parity,filters,widths,pdfPages:pages,downloadMatchesArchive:true,fixtures,consoleErrors:exceptions};
 await fs.writeFile(path.join(output,'verification.json'),JSON.stringify(result,null,2));console.log(JSON.stringify(result));
 await call('Browser.close');socket.close();
 assert(parity.every(p=>p.dimensions&&p.different===0),'Visual parity differs; inspect saved screenshots and verification.json');
}catch(error){console.error(error);child.kill();if(socket)socket.close();process.exitCode=1;}
