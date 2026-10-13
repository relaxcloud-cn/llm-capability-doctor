import argparse
import copy
import hashlib
import json
from pathlib import Path
import shutil
import subprocess

parser=argparse.ArgumentParser(description='Create report UI fixtures without model calls or modifications to the original run.')
parser.add_argument('--result',type=Path,required=True)
parser.add_argument('--output',type=Path,required=True)
parser.add_argument('--binary',type=Path,default=Path('target/debug/agent-check'))
parser.add_argument('--producer-version',help='Optional known detector version for the copied replay only; never inferred from the renderer.')
args=parser.parse_args()
ROOT=Path(__file__).resolve().parents[1]
source=args.result.resolve().parent
output=args.output.resolve()
binary=args.binary.resolve()
if source==output or source in output.parents:
    parser.error('The output directory must be separate from the source run')
output.mkdir(parents=True,exist_ok=False)
result=json.loads(args.result.read_text())
original_hashes={str(p.relative_to(source)):hashlib.sha256(p.read_bytes()).hexdigest() for p in source.rglob('*') if p.is_file() and not p.is_symlink()}
actual=output/'live'
shutil.copytree(source,actual,symlinks=True,ignore=shutil.ignore_patterns('report.html','records.zip','config'))
replay=copy.deepcopy(result)
if args.producer_version:
    replay['app_version']=args.producer_version
(actual/'result.json').write_text(json.dumps(replay,ensure_ascii=False,indent=2)+'\n')
subprocess.run([str(binary),'--render-result',str(actual/'result.json')],check=True)
rendered=json.loads((actual/'result.json').read_text())
for key in ['verdict','checks','evidence','plan','decision','check_version','started_at','cancelled']:
    assert rendered.get(key)==result.get(key), key+' changed during report generation'
fixtures=[]
for case in ['mixed','incomplete','empty','legacy','malicious']:
    directory=output/case
    directory.mkdir()
    fixture=copy.deepcopy(result)
    fixture['id']='UI-FIXTURE-'+case
    fixture['target']['model']='UI 状态验收样本 · '+case+'（非真实检测）'
    fixture['target']['endpoint']='https://example.org/v1/chat/completions'
    fixture['evidence']={}
    fixture.pop('decision',None)
    fixture['verdict']='passed'
    fixture['cancelled']=False
    for check in fixture['checks']:
        check.update(status='passed',finished=True,evidence_ids=[],tasks=[],facts={},findings=[],title='本次展示样本通过')
        check.pop('assessment',None)
    if case=='mixed':
        fixture['verdict']='failed'
        fixture['checks'][0].update(status='failed',title='服务请求返回 HTTP 503')
        fixture['checks'][0]['findings']=[{'code':'ui_service_error','title':'服务请求返回 HTTP 503','fact':'受控展示样本：服务请求未完成。','impact':'本次接口要求未满足，不代表模型本体没有相关能力。','suggestion':'核对服务可用性后重新检测。','origin':'service','evidence_ids':[]}]
        fixture['checks'][1].update(status='incomplete',title='证据不足，工具能力未确认')
        fixture['checks'][2].update(status='not_executed',title='未执行',finished=False)
    elif case=='incomplete':
        fixture['verdict']='incomplete';fixture['cancelled']=True
        fixture['checks'][0].update(status='incomplete',title='检测过程被取消，结果未确认')
        for check in fixture['checks'][1:]:
            check.update(status='not_executed',finished=False,title='尚未执行')
    elif case=='empty':
        fixture.update(verdict='incomplete',checks=[],plan=[])
    elif case=='legacy':
        fixture.pop('app_version',None)
    elif case=='malicious':
        fixture['target']['model']='</script><img src=x onerror="window.injected=true">{{ROWS}}'
        fixture['checks'][0]['title']='<svg onload="window.injected=true"></svg> {{MODEL}}'
    (directory/'result.json').write_text(json.dumps(fixture,ensure_ascii=False,indent=2)+'\n')
    subprocess.run([str(binary),'--render-result',str(directory/'result.json')],check=True)
    fixtures.append({'case':case,'report':str(directory/'report.html')})
assert original_hashes=={str(p.relative_to(source)):hashlib.sha256(p.read_bytes()).hexdigest() for p in source.rglob('*') if p.is_file() and not p.is_symlink()}
receipt={'output':str(output),'live_report':str(actual/'report.html'),'reference':str(ROOT/'docs/report-reference/agentcheck-customer-report.html'),'fixtures':fixtures,'original_run_untouched':True,'producer_version_override':args.producer_version,'source_result':str(args.result.resolve())}
(output/'render-receipt.json').write_text(json.dumps(receipt,ensure_ascii=False,indent=2)+'\n')
print(json.dumps(receipt,ensure_ascii=False))
