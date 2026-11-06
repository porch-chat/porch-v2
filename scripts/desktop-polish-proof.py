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
        community=cli('community-create','Weekend Crew')['id']
        def mutate(**body): return cli('community-mutate',community,PORCH_COMMAND=json.dumps(body))['id']
        category=mutate(op='create_category',name='Text Channels',position=0)
        for i,name in enumerate(['general','random','dev']):
            mutate(op='create_channel',name=name,kind='text',category_id=category,position=i,synced=True)
        voicecat=mutate(op='create_category',name='Voice Channels',position=1)
        mutate(op='create_channel',name='General',kind='voice',category_id=voicecat,position=0,synced=True)
        mutate(op='create_channel',name='Gaming',kind='voice',category_id=voicecat,position=1,synced=True)
        invite=cli('invite-create',5,3600)['code']
        join=cli('community-invite-create',community,5,3600)['code']
        tokens=[admin['token']]
        for name in ['maya','jordan','sam']:
            auth=cli('register',PORCH_USERNAME=name,PORCH_PASSWORD=password,PORCH_INVITE=invite)
            tokens.append(auth['token'])
            cli('community-join',PORCH_TOKEN=auth['token'],PORCH_COMMUNITY_INVITE=join)
        heads=cli('conversations'); head=next(h for h in heads if h.get('community_id')==community and h['name']=='general')
        lines=[(0,'Is the build passing on main now?'),(0,'Looks like the Windows job is still running.'),(1,'Yes, it went green about ten minutes ago.'),(2,'Nice. I will write the release notes this afternoon.'),(2,'Draft is in the shared doc if anyone wants to read it first.'),(3,'Reading it now. Section two needs a link to the install steps.'),(0,'Added it, thanks.')]
        for i,(who,text) in enumerate(lines):
            cli('message-send',head['id'],PORCH_TOKEN=tokens[who],PORCH_MESSAGE=json.dumps({'nonce':f'polish_fixture_{i:016}','content':{'format':'markdown','text':text},'reply_to':None}))
        fixture=Path(data)/'release-notes.txt'; fixture.write_text('Release notes draft.\n')
        file=cli('file-upload',head['id'],fixture)
        cli('message-send',head['id'],PORCH_MESSAGE=json.dumps({'nonce':'polish_attachment_00000001','content':{'format':'markdown','text':'Latest draft attached.'},'attachments':[file['id']],'reply_to':None}))
        subprocess.run(['psql','-h','127.0.0.1','-p','35415','-U','porch','-d',os.environ['DATABASE_URL'].rsplit('/',1)[1],'-c',"WITH r AS (SELECT id,row_number() OVER (ORDER BY id) n,date_trunc('minute',now())-interval '6 hours'+(ARRAY[0,1,15,47,49,95,130,131])[row_number() OVER (ORDER BY id)]*interval '1 minute' t FROM messages WHERE conversation_id=%d) UPDATE messages m SET created_at=r.t,data=jsonb_set(m.data,'{created_at}',to_jsonb(r.t::text)) FROM r WHERE m.id=r.id"%head['id']],env={**os.environ,'PGPASSWORD':os.environ.get('PGPASSWORD','')},check=True)
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
        subprocess.run(['xdotool','mousemove','52','318','click','1'],env=ui,check=True)
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
