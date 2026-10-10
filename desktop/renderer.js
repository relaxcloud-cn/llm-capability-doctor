'use strict';
const api=window.agentcheck,$=id=>document.getElementById(id);
const ids=['response','tools','schema','context','performance','continuous','recovery','multiturn'];
const names=['接口与响应','工具调用','结构化输出','长上下文','并发性能测试','连续工具执行','工具故障恢复','多轮继续执行'];
const esc=value=>String(value??'').replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const clock=ms=>{const total=Math.max(0,Math.floor(ms/1000));return `${String(Math.floor(total/60)).padStart(2,'0')}:${String(total%60).padStart(2,'0')}`;};
const date=value=>new Intl.DateTimeFormat('sv-SE',{timeZone:'Asia/Shanghai',year:'numeric',month:'2-digit',day:'2-digit',hour:'2-digit',minute:'2-digit',hour12:false}).format(new Date(value));
const icon=status=>`<span class="state-icon ${status==='passed'?'passed':status==='failed'?'failed':status==='running'?'running':'waiting'}">${status==='failed'?'!':`<svg aria-hidden="true"><use href="#${status==='passed'?'check':status==='running'?'play':'minus'}"/></svg>`}</span>`;
const label=status=>({passed:'通过',failed:'未通过',incomplete:'未确认',not_executed:'未执行',running:'进行中'})[status]||'未执行';
const initialInputs=$('live-checklist').innerHTML;
let run=null,busy=false,records=[],scope='all',expanded=null,stopping=false,connected=false,initialized=false,sequence=0,queued=[];
let liveStart=null,elapsed=0,reporting=false,lastError=null;
const active=new Map(),steps=new Map(),started=new Set(),taskDetails=new Map(),taskOpen=new Map();
function currentCheck(id){return run?.checks?.find(check=>check.id===id)||{id,status:'not_executed',finished:false,title:'等待执行',tasks:[]};}
function passedCount(r){return r.checks.filter(check=>check.status==='passed').length;}
function failedCount(r){return r.checks.filter(check=>check.status==='failed').length;}
function completedCount(r){return r?.checks?.filter(check=>check.finished).length||0;}
function verdict(r){return r.verdict==='passed'?'本次通过接入验收':r.verdict==='failed'?'暂不建议接入平台':'检测未完成';}
function mainReason(r){const check=r.checks.find(check=>check.status==='failed')||r.checks.find(check=>check.status!=='passed');return check?`${names[ids.indexOf(check.id)]}：${check.title}`:'本次全部必测项目均满足要求。';}
function isRunning(){return busy&&run&&!run.finished_at;}
function readOnlyButtons(){document.querySelectorAll('[data-new]').forEach(button=>{button.disabled=busy;button.title=busy?'本次检测和报告生成结束后可新建':'新建检测';});$('retest').disabled=busy;}
function page(){const history=location.hash==='#history'||!run;$('workbench-page').hidden=history;$('history-page').hidden=!history;
 $('toolbar-state').textContent=history?'检测记录':isRunning()?'检测进行中':run?.cancelled?'检测已停止':'检测已完成';$('toolbar-id').textContent=history?'本机记录':run?.id||'';
 document.querySelectorAll('[data-page]').forEach(button=>{const selected=button.dataset.page===(history?'history':'workbench');button.classList.toggle('active',selected&&button.classList.contains('nav-item'));if(selected)button.setAttribute('aria-current','page');else button.removeAttribute('aria-current');});
 if(history)renderHistory();
}
function renderHistory(){
 $('history-count').textContent=records.length;
 $('ongoing-banner').hidden=!isRunning();if(isRunning()){$('ongoing-banner').querySelector('strong').textContent=run.target.model+' 正在检测';$('ongoing-banner').querySelector('p').textContent=`已完成 ${completedCount(run)} / 8 个检测项 · 已用时 ${clock(elapsed)}`;}
 const search=$('history-search').value.trim().toLowerCase();
 const rows=records.filter(record=>(scope==='all'||run&&record.target.model===run.target.model&&record.target.endpoint===run.target.endpoint)&&(!search||(record.target.model+' '+record.target.endpoint).toLowerCase().includes(search)));
 $('history-caption').textContent=search||scope!=='all'?`显示 ${rows.length} / ${records.length} 条记录`:`共 ${records.length} 条已保存记录`;
 $('record-list').hidden=!rows.length;$('empty-records').hidden=Boolean(rows.length);
 $('empty-records').querySelector('h2').textContent=records.length?'没有找到匹配记录':'还没有检测记录';$('empty-records').querySelector('p').textContent=records.length?'试试其他模型名称或服务地址。':'完成检测后，结果会保存在这里。';$('clear-filters').hidden=!records.length;
 $('history-records').innerHTML=rows.map(record=>{
  const partial=record.cancelled,open=expanded===record.id,failed=failedCount(record),missing=8-passedCount(record)-failed;
  return `<tr class="record-line"><td class="model-cell"><span class="record-model" title="${esc(record.target.endpoint)} · ${esc(record.id)}">${esc(record.target.model)}</span><time class="record-time num">${date(record.started_at)}</time></td><td class="verdict-cell"><span class="record-verdict ${record.verdict==='failed'?'fail':''}"><i class="verdict-dot" aria-hidden="true"></i>${verdict(record)}</span><p class="record-reason">${esc(partial&&record.verdict!=='failed'?'用户提前停止，已保留完成结果':mainReason(record))}</p></td><td class="count-cell"><div class="record-count num">${completedCount(record)} <small>/ 8 项</small></div><p class="record-count-note">${failed?`${failed} 项未通过`:missing?`${missing} 项未确认或未完成`:'全部通过'}</p></td><td class="duration-cell"><span class="record-duration num">${clock(record.elapsed_ms)}</span></td><td class="actions-cell"><div class="record-buttons">${partial?`<button class="text-button" data-record="${record.id}" aria-expanded="${open}">${open?'收起结果 ↑':'查看结果 ↓'}</button>`:record.report_path?`<button class="report-link" data-download="${record.id}">下载</button><button class="report-link primary" data-report="${record.id}">查看报告 ↗</button>`:'<span class="record-reason">报告未生成</span>'}</div></td></tr>${partial?`<tr class="partial-row" ${open?'':'hidden'}><td colspan="5"><section class="partial-results"><h3>本次保留的检测结果</h3><p class="record-detail-meta">${esc(record.target.endpoint)} · ${record.id}</p>${record.checks.map(check=>`<div class="partial-result"><span class="partial-name">${names[ids.indexOf(check.id)]}</span><span class="partial-conclusion ${check.status==='passed'?'':check.status==='failed'?'failure-text':'unfinished'}">${esc(check.title)}</span></div>`).join('')}</section></td></tr>`:''}`;
 }).join('');
}
function renderCompleted(){
 const body=$('completion-overview');body.classList.toggle('passed-overview',run.verdict==='passed');body.classList.toggle('incomplete-overview',run.verdict==='incomplete');
 $('completion-verdict').textContent=verdict(run);body.querySelector('.completion-mark').textContent=run.verdict==='passed'?'✓':run.verdict==='failed'?'!':'—';
 body.querySelector('.completion-lead p').textContent=mainReason(run)+(run.verdict==='passed'?' 建议按本次验证的配置接入。':' 建议处理问题或补齐未完成检测后重测。');
 body.querySelector('.completion-meta').innerHTML=`<span>已完成 <b class="num">${completedCount(run)} / 8</b> 项</span><span><b class="pass-count">${passedCount(run)} 项通过</b> · <b class="fail-count">${failedCount(run)} 项未通过</b>${8-passedCount(run)-failedCount(run)?` · ${8-passedCount(run)-failedCount(run)} 项未确认或未完成`:''}</span><span>总用时 <b class="num">${clock(run.elapsed_ms)}</b></span><span class="saved-status">记录已保存${run.report_path?'，报告已生成':'，报告未生成'}</span>`;
 let result='<div class="group-title"><h2><svg aria-hidden="true"><use href="#chip"/></svg>模型接口检测</h2><span>5 项</span></div>';
 for(const id of ids.slice(0,5)){const check=currentCheck(id);result+=`<div class="interface-row"><div class="check-name">${icon(check.status)}${names[ids.indexOf(id)]}</div><p class="conclusion ${check.status==='failed'?'failure-text':check.status!=='passed'?'unknown-text':''}">${esc(check.title)}</p><span class="result-label">${label(check.status)}</span></div>`;}
 result+='<div class="group-title agent-group"><h2><svg aria-hidden="true"><use href="#layers"/></svg>智能体任务检测</h2><span>3 类，每类 2 次</span></div>';
 for(const id of ids.slice(5)){const check=currentCheck(id);result+=`<details class="complete-task ${check.status==='failed'?'is-failed':''}" ${check.status==='failed'?'open':''}><summary><span class="check-name">${icon(check.status)}${names[ids.indexOf(id)]}</span><span class="conclusion ${check.status!=='passed'?'unknown-text':''}">${esc(check.title)}</span><span class="result-label">${label(check.status)}<svg class="chevron" aria-hidden="true"><use href="#chevron"/></svg></span></summary><div class="complete-task-body">${[1,2].map(repetition=>{const task=check.tasks.find(task=>task.repetition===repetition);return `<div class="complete-run ${task?.status==='failed'?'failed':''}"><span>第 ${repetition} 次执行</span><span class="run-result ${task&&task.status!=='passed'?'unknown-text':''}">${esc(task?.title||'未执行')}</span><span class="run-label">${label(task?.status)}</span></div>`;}).join('')}${check.status==='failed'&&run.report_path?`<div class="problem-fact"><button class="text-button" data-report="${run.id}">查看问题与建议 ↗</button></div>`:''}</div></details>`;}
 $('completed-checklist').innerHTML=result;
}
function renderLive(){
 const stopped=run.cancelled&&!busy;
 $('phase-title').textContent=stopped?'本次检测已停止':stopping?'正在停止并保留结果':reporting?'正在生成报告和整理检测记录':started.size?`正在检测${started.has('continuous')||started.has('recovery')||started.has('multiturn')?'智能体任务能力':'模型接口能力'}`:'准备检测';
 const percent=Math.round(completedCount(run)/8*100);document.querySelector('.percentage').innerHTML=`${percent}<span>%</span>`;document.querySelector('.progress-fill').style.width=percent+'%';document.querySelector('[role="progressbar"]').setAttribute('aria-valuenow',String(percent));document.querySelector('.meta-count strong').textContent=`${completedCount(run)} / 8`;document.querySelector('.meta-right b').textContent=clock(elapsed);$('active-count').textContent=stopped?`${8-completedCount(run)} 个检测项未完成`:active.size?`${active.size} 个任务同时执行`:reporting?'正在整理报告':'按顺序执行检测';
 $('live-checklist').innerHTML=initialInputs;$('live-checklist').querySelector('.group-title>span').textContent=`${run.checks.slice(0,5).filter(check=>check.finished).length} / 5 项已完成`;
 document.querySelectorAll('#live-checklist .interface-row').forEach((row,index)=>{const id=ids[index],check=currentCheck(id),running=started.has(id)&&!check.finished;row.querySelector('.state-icon').outerHTML=icon(running?'running':check.status);row.querySelector('.conclusion').textContent=running?steps.get(id)||'检查中':check.title;row.querySelector('.conclusion').classList.toggle('failure-text',check.status==='failed');row.querySelector('.conclusion').classList.toggle('unknown-text',!running&&check.status!=='passed'&&check.status!=='failed');row.querySelector('.result-label').textContent=running?'进行中':label(check.status);});
 const agentStart=$('live-checklist').querySelector('.agent-group');let agentHtml='';
 for(const id of ids.slice(5)){
  const check=currentCheck(id),liveTasks=[1,2].map(repetition=>taskDetails.get(`${id}-${repetition}`)),running=liveTasks.some(task=>task?.running),done=check.tasks.filter(task=>task.finished).length;
  const status=running?'进行中':check.finished?label(check.status):stopped?(check.tasks.length?'已停止':'未执行'):done?'等待下次执行':'等待执行';
  agentHtml+=`<details class="task" data-task-kind="${id}" ${(taskOpen.has(id)?taskOpen.get(id):running)?'open':''}><summary><span class="check-name">${icon(running?'running':check.status)}${names[ids.indexOf(id)]}</span><span class="task-status"><span class="tag">${status}</span><span>已完成 ${done} / 2 次</span></span><svg class="chevron" aria-hidden="true"><use href="#chevron"/></svg></summary><div class="task-body">${[1,2].map(repetition=>{const task=liveTasks[repetition-1],finished=check.tasks.find(task=>task.repetition===repetition);return `<div class="execution-line"><span class="execution-label">第 ${repetition} 次</span><span class="execution-step"><i class="step-dot"></i>${esc(task?.running?task.step:finished?.title||(stopped?'未执行':'等待执行'))}</span><span class="execution-time num">${task?`已执行 ${clock(task.running?elapsed-task.started:finished?.elapsed_ms||0)}`:''}</span></div>`;}).join('')}${id==='recovery'?'<p class="test-note">模拟一次工具读取故障，检查能否继续完成任务。</p>':''}</div></details>`;
 }
 let node=agentStart.nextElementSibling;while(node){const next=node.nextElementSibling;node.remove();node=next;}agentStart.insertAdjacentHTML('afterend',agentHtml);
}
function render(){readOnlyButtons();
 document.querySelector('.model-card strong').textContent=run?.target.model||'尚未配置模型';$('nav-state').hidden=!run;$('nav-state').textContent=isRunning()?'检测中':run?.cancelled?'已停止':'已完成';
 document.querySelector('.connection').innerHTML=`<i class="dot" style="background:${connected?'var(--green)':'#a1aab8'}"></i>${connected?'模型服务已连接':'尚未连接模型服务'}`;
 if(run){$('workbench-page').querySelector('h1').textContent=run.target.model;$('workbench-page').querySelector('.heading p').textContent=`${run.target.endpoint} · ${date(run.started_at)} 开始`;
 const stopped=Boolean(run.cancelled&&run.finished_at),complete=Boolean(run.finished_at)&&!stopped;
 $('app').classList.toggle('stopped',stopped);$('app').classList.toggle('completed',complete);
 $('stop').hidden=!isRunning();$('stop').disabled=stopping;$('completion-actions').hidden=!complete;$('completion-overview').hidden=!complete;$('completed-results').hidden=!complete;document.querySelector('.progress-area').hidden=complete;$('live-checklist').hidden=complete;$('retest-note').hidden=true;
 $('view-complete-report').hidden=!run.report_path;$('view-complete-report').removeAttribute('href');$('view-complete-report').dataset.report=run.id;
 if(complete)renderCompleted();else renderLive();
 document.querySelector('.stopped-note').textContent=`本次检测已停止，${completedCount(run)} 个已完成检测项的结果已保留。其余项目未完成，不计为模型失败。`;
 $('footer-text').textContent=lastError|| (busy&&run.finished_at?'正在检查模型连接，通过后开始新的检测。':complete?'本次结果已保存到检测记录，可随时查看和下载报告。':stopped?'已完成结果已保存到检测记录。':reporting?'正在生成报告，文件保存后即可查看。':'检测结束后生成完整报告；停止检测会保留已完成结果。');
 }page();
}
async function refreshHistory(){try{records=await api.history();renderHistory();}catch(error){showError(error.message);}}
function event(message){
 if(message.sequence&&message.sequence<=sequence)return;if(message.sequence)sequence=message.sequence;
 if(message.type==='state'){busy=Boolean(message.busy);if(!busy){stopping=false;refreshHistory();}}
 else if(message.type==='connection'){connected=true;}
 else if(message.type==='error'){showError(message.message);}
 else if(message.type==='event'){
  const envelope=message.data,ev=envelope.event;elapsed=envelope.elapsed_ms;
  if(ev.type==='started'){lastError=null;run=ev.run;busy=true;stopping=false;reporting=false;liveStart=performance.now()-elapsed;active.clear();steps.clear();started.clear();taskDetails.clear();taskOpen.clear();location.hash='workbench';}
  else if(ev.type==='check_started')started.add(ev.id);
  else if(ev.type==='step')steps.set(ev.id,ev.message);
  else if(ev.type==='task_started'){active.set(ev.id,ev);taskDetails.set(`${ev.kind}-${ev.repetition}`,{running:true,started:elapsed,step:'启动运行环境'});}
  else if(ev.type==='task_step'){const detail=taskDetails.get(`${ev.kind}-${ev.repetition}`);if(detail)detail.step=ev.message;}
  else if(ev.type==='task_finished'){active.delete(ev.task.id);const detail=taskDetails.get(`${ev.task.kind}-${ev.task.repetition}`);if(detail)detail.running=false;}
  else if(ev.type==='check_finished'||ev.type==='check_progress'){if(run){const index=run.checks.findIndex(check=>check.id===ev.check.id);run.checks[index]=ev.check;}if(ev.check.finished)started.delete(ev.check.id);}
  else if(ev.type==='report_started')reporting=true;
  else if(ev.type==='error')showError(ev.message);
  else if(ev.type==='finished'){run=ev.run;elapsed=run.elapsed_ms;reporting=false;active.clear();started.clear();liveStart=null;refreshHistory();}
 }render();
}
function showError(message){lastError=message;$('footer-text').textContent=message;}
function destination(){
 let url;try{url=new URL($('service-url').value.trim());}catch{return null;}
 const choice=$('protocol-choice').value==='chat'?'chat-completions':$('protocol-choice').value,path=url.pathname.replace(/\/+$/,'');const detected=path.endsWith('/responses')?'responses':path.endsWith('/chat/completions')?'chat-completions':null;
 if(detected&&choice!=='auto'&&choice!==detected)return null;
 const protocol=choice==='auto'?(detected||'chat-completions'):choice;
 return {protocol,endpoint:url.origin+(detected?path:(path||'/v1')+(protocol==='responses'?'/responses':'/chat/completions'))};
}
function preview(){const target=destination();$('request-preview').hidden=!target;if(target){$('resolved-protocol').textContent=target.protocol==='responses'?'Responses':'Chat Completions';$('resolved-endpoint').textContent=target.endpoint;}}
function openNew(){if(busy)return;$('service-url').value=run?.target.endpoint||'';$('model-name').value=run?.target.model||'';$('api-key').value='';$('api-key').type='password';$('protocol-choice').value='auto';$('connection-notice').hidden=true;document.querySelectorAll('.field-error').forEach(node=>node.hidden=true);preview();$('new-dialog').showModal();}
function closeNew(){if(busy&&!isRunning())api.cancel().catch(()=>{});$('api-key').value='';$('api-key').type='password';$('new-dialog').close();}
function formBusy(value){$('config-fields').disabled=value;$('start-new').disabled=value;$('start-new').textContent=value?'正在检查连接…':'检查连接并开始';}
function connectionError(message){$('connection-notice').hidden=false;$('connection-notice').classList.remove('checking');$('connection-title').textContent='连接检查未通过';$('connection-notice').querySelector('.notice-mark').textContent='!';$('connection-description').textContent=message;}
$('new-form').addEventListener('submit',async e=>{e.preventDefault();if(busy)return;let invalid=false;for(const [input,error,text]of[['service-url','service-error','请填写完整的服务地址。'],['model-name','model-error','请填写模型名称。'],['api-key','key-error','请输入 API Key。']]){const missing=!$(input).value.trim();$(error).hidden=!missing;$(error).textContent=text;invalid||=missing;}
 if(!$('service-url').value.trim()||!destination()){connectionError('请核对完整服务地址与所选接口协议。');return;}if(invalid)return;
 formBusy(true);$('connection-notice').hidden=false;$('connection-notice').classList.add('checking');$('connection-title').textContent='正在检查模型连接';$('connection-notice').querySelector('.notice-mark').textContent='';$('connection-description').textContent='连接成功后将自动开始检测。';
 try{await api.submit({base_url:$('service-url').value.trim(),model:$('model-name').value.trim(),api_key:$('api-key').value,protocol:$('protocol-choice').value==='auto'?null:$('protocol-choice').value==='chat'?'chat-completions':$('protocol-choice').value});closeNew();}
 catch(error){connectionError(error.message);}
 finally{formBusy(false);}
});
$('cancel-new').addEventListener('click',closeNew);$('new-dialog').addEventListener('cancel',e=>{e.preventDefault();closeNew();});
$('toggle-key').addEventListener('click',()=>{const show=$('api-key').type==='password';$('api-key').type=show?'text':'password';$('toggle-key').textContent=show?'隐藏':'显示';$('toggle-key').setAttribute('aria-pressed',String(show));});
$('service-url').addEventListener('input',preview);$('protocol-choice').addEventListener('change',preview);
$('stop').addEventListener('click',()=>$('stop-dialog').showModal());$('keep-running').addEventListener('click',()=>$('stop-dialog').close());
$('confirm-stop').addEventListener('click',async()=>{$('stop-dialog').close();stopping=true;render();try{await api.cancel();}catch(error){showError(error.message);}});
$('retest').addEventListener('click',async()=>{if(busy)return;try{await api.retest();}catch(error){showError(error.message);}});
for(const button of document.querySelectorAll('[data-page]'))button.addEventListener('click',()=>{location.hash=button.dataset.page;page();});
for(const button of document.querySelectorAll('[data-new]'))button.addEventListener('click',openNew);
for(const button of document.querySelectorAll('[data-scope]'))button.addEventListener('click',()=>{scope=button.dataset.scope;document.querySelectorAll('[data-scope]').forEach(node=>node.setAttribute('aria-pressed',String(node===button)));renderHistory();});
$('history-search').addEventListener('input',renderHistory);$('clear-filters').addEventListener('click',()=>{scope='all';$('history-search').value='';document.querySelectorAll('[data-scope]').forEach(node=>node.setAttribute('aria-pressed',String(node.dataset.scope==='all')));renderHistory();});
document.addEventListener('click',async e=>{const report=e.target.closest('[data-report]'),download=e.target.closest('[data-download]'),record=e.target.closest('[data-record]');try{if(report){e.preventDefault();await api.report(report.dataset.report);}else if(download){await api.download(download.dataset.download);}else if(record){expanded=expanded===record.dataset.record?null:record.dataset.record;renderHistory();}}catch(error){showError(error.message);}});
document.addEventListener('toggle',e=>{if(e.target.matches?.('details[data-task-kind]'))taskOpen.set(e.target.dataset.taskKind,e.target.open);},true);
window.addEventListener('hashchange',page);
api.subscribe(message=>{if(!initialized)queued.push(message);else event(message);});
(async()=>{try{const state=await api.state();run=state.run;busy=state.busy;connected=Boolean(run);for(const message of state.events||[])event(message);sequence=Math.max(sequence,state.snapshot_sequence||0);records=await api.history();initialized=true;for(const message of queued)event(message);queued=[];if(!run)location.hash='history';render();await api.ready();}catch(error){showError(error.message);}})();
setInterval(()=>{if(liveStart&&busy){elapsed=performance.now()-liveStart;render();}},500);
