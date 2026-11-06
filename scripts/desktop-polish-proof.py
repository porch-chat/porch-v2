#!/usr/bin/env python3
"""Native polish evidence against a fresh disposable database, never production.
All users/messages are generated fixtures through actual protocol endpoints.
Requires DISPLAY at 1440x1000, DATABASE_URL pointing at an empty DB.
"""
import json, os, secrets, socket, subprocess, tempfile, time, urllib.request
from pathlib import Path
root = Path(__file__).resolve().parent.parent
env = os.environ.copy()
assert env.get('DISPLAY') and env.get('DATABASE_URL')
bin = root / 'target/debug'
password = secrets.token_hex(24)
subprocess.run([bin/'porch-server','bootstrap-admin'], env={**env,'PORCH_BOOTSTRAP_USERNAME':'dylan','PORCH_BOOTSTRAP_PASSWORD':password}, capture_output=True, check=True)
with socket.socket() as s:
    s.bind(('127.0.0.1',0)); port=s.getsockname()[1]
env.update(PORCH_BIND=f'127.0.0.1:{port}', PORCH_URL=f'http://127.0.0.1:{port}')
def cli(*args, **extra):
    p=subprocess.run([bin/'porch-cli',*map(str,args)],env={**env,**extra},capture_output=True,text=True)
    if p.returncode: raise RuntimeError(p.stderr)
    return json.loads(p.stdout)
server=desktop=wm=None
with tempfile.TemporaryDirectory(prefix='porch-polish-') as data:
    try:
        env.update(PORCH_STORAGE_ROOT=data+'/files')
        server=subprocess.Popen([bin/'porch-server'],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        for _ in range(100):
            try:
                urllib.request.urlopen(env['PORCH_URL']+'/health',timeout=1); break
            except OSError: time.sleep(.1)
        admin=cli('login',PORCH_USERNAME='dylan',PORCH_PASSWORD=password)
        env['PORCH_TOKEN']=admin['token']
        community=cli('community-create','The Evening Porch')['id']
        def mutate(**body): return cli('community-mutate',community,PORCH_COMMAND=json.dumps(body))['id']
        category=mutate(op='create_category',name='Around the porch',position=0)
        for i,name in enumerate(['general','weekend-plans','photos-and-finds']):
            mutate(op='create_channel',name=name,kind='text',category_id=category,position=i,synced=True)
        voicecat=mutate(op='create_category',name='Pull up a chair',position=1)
        mutate(op='create_channel',name='Front steps',kind='voice',category_id=voicecat,position=0,synced=True)
        mutate(op='create_channel',name='Game night',kind='voice',category_id=voicecat,position=1,synced=True)
        invite=cli('invite-create',5,3600)['code']
        join=cli('community-invite-create',community,5,3600)['code']
        tokens=[admin['token']]
        for name in ['lily','zach','amber']:
            auth=cli('register',PORCH_USERNAME=name,PORCH_PASSWORD=password,PORCH_INVITE=invite)
            tokens.append(auth['token'])
            cli('community-join',PORCH_TOKEN=auth['token'],PORCH_COMMUNITY_INVITE=join)
        heads=cli('conversations'); head=next(h for h in heads if h.get('community_id')==community and h['name']=='general')
        lines=[(0,'Made it home. The porch light is officially on.'),(0,'Anyone around for a slow evening?'),(1,'Just put the kettle on. Give me five minutes.'),(2,'I found the perfect spot for game night.'),(2,'No complicated plans. Just us, something warm, and a few games.'),(0,'That sounds like exactly the right amount of planning.'),(1,'Saving a chair for everyone.')]
        for i,(who,text) in enumerate(lines):
            cli('message-send',head['id'],PORCH_TOKEN=tokens[who],PORCH_MESSAGE=json.dumps({'nonce':f'polish_fixture_{i:016}','content':{'format':'markdown','text':text},'reply_to':None}))
        fixture=Path(data)/'game-night.txt'; fixture.write_text('Tea, warm lights, and one more round.\n')
        file=cli('file-upload',head['id'],fixture)
        cli('message-send',head['id'],PORCH_MESSAGE=json.dumps({'nonce':'polish_attachment_00000001','content':{'format':'markdown','text':'The very serious game-night agenda.'},'attachments':[file['id']],'reply_to':None}))
        ui={**env,'PORCH_DATA_DIR':data+'/cache','PORCH_SMOKE_SECONDS':'75','VK_ICD_FILENAMES':'/usr/share/vulkan/icd.d/lvp_icd.json'}
        wm=subprocess.Popen(['openbox'],env=ui,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        desktop=subprocess.Popen([bin/'porch-desktop'],env=ui,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        subprocess.run(['xdotool','search','--sync','--name','^Porch$'],env=ui,timeout=20,stdout=subprocess.DEVNULL)
        time.sleep(3)
        for y,value in [(466,env['PORCH_URL']),(519,'dylan'),(571,password)]:
            subprocess.run(['xdotool','mousemove','720',str(y),'click','1','key','ctrl+a'],env=ui,check=True)
            subprocess.run(['xdotool','type','--clearmodifiers','--file','-'],input=value,text=True,env=ui,check=True)
        subprocess.run(['xdotool','mousemove','720','622','click','1'],env=ui,check=True)
        time.sleep(6)
        # Official community is first, the fixture community second.
        subprocess.run(['xdotool','mousemove','52','310','click','1'],env=ui,check=True)
        time.sleep(4)
        subprocess.run(['xdotool','mousemove','700','110'],env=ui,check=True)
        time.sleep(2)
        out=root/'docs/evidence'; out.mkdir(exist_ok=True)
        # Capture the native client, excluding Openbox's bright external frame.
        wid=subprocess.check_output(['xdotool','search','--name','^Porch$'],env=ui,text=True).splitlines()[0]
        subprocess.run(['import','-window',wid,out/'ui-polish-conversation.png'],env=ui,check=True)
        subprocess.run(['xdotool','mousemove','700','390'],env=ui,check=True)
        time.sleep(1)
        subprocess.run(['import','-window',wid,out/'ui-polish-hover.png'],env=ui,check=True)
        print('Native fixture screenshots captured; inspect before claiming acceptance.')
    finally:
        for p in (desktop,wm,server):
            if p and p.poll() is None:
                p.terminate()
                try: p.wait(timeout=5)
                except subprocess.TimeoutExpired: p.kill(); p.wait()
