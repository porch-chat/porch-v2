#!/usr/bin/env python3
"""Native redesign evidence against a disposable database; generated fixtures through real endpoints.
Needs DATABASE_URL (server role; a scratch database is recreated), Xvfb, openbox, xdotool, ImageMagick.
Writes docs/evidence/redesign-{group,home,dm}.png at 1920x1080."""
import json, os, secrets, socket, subprocess, tempfile, time, urllib.request, urllib.parse
from pathlib import Path
root = Path(__file__).resolve().parent.parent
env = os.environ.copy()
bin = root / 'target/debug'
base = urllib.parse.urlparse(env['DATABASE_URL'])
dbname = 'porch_redesign_proof'
admin_db = base._replace(path='/postgres').geturl()
for sql in (f'DROP DATABASE IF EXISTS {dbname}', f'CREATE DATABASE {dbname}'):
    subprocess.run(['psql', admin_db, '-qc', sql], check=True)
env['DATABASE_URL'] = base._replace(path='/' + dbname).geturl()
password = secrets.token_hex(24)
subprocess.run([bin/'porch-server','bootstrap-admin'], env={**env,'PORCH_BOOTSTRAP_USERNAME':'sam','PORCH_BOOTSTRAP_PASSWORD':password}, capture_output=True, check=True)
with socket.socket() as s:
    s.bind(('127.0.0.1',0)); port=s.getsockname()[1]
with socket.socket() as s:
    s.bind(('127.0.0.1',0)); dnum=60+s.getsockname()[1]%30
env.update(PORCH_BIND=f'127.0.0.1:{port}', PORCH_URL=f'http://127.0.0.1:{port}')
def cli(*args, **extra):
    p=subprocess.run([bin/'porch-cli',*map(str,args)],env={**env,**extra},capture_output=True,text=True)
    if p.returncode: raise RuntimeError(p.stderr)
    return json.loads(p.stdout)
def send(head, text, token, n):
    return cli('message-send',head,PORCH_TOKEN=token,PORCH_MESSAGE=json.dumps({'nonce':f'redesign_{n:020}','content':{'format':'markdown','text':text},'reply_to':None}))
server=desktop=wm=xvfb=None
with tempfile.TemporaryDirectory(prefix='porch-redesign-') as data:
    try:
        env.update(PORCH_STORAGE_ROOT=data+'/files')
        server=subprocess.Popen([bin/'porch-server'],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        for _ in range(100):
            try: urllib.request.urlopen(env['PORCH_URL']+'/health',timeout=1); break
            except OSError: time.sleep(.1)
        sam=cli('login',PORCH_USERNAME='sam',PORCH_PASSWORD=password)
        env['PORCH_TOKEN']=sam['token']
        community=cli('community-create','Friends')['id']
        def mutate(**body): return cli('community-mutate',community,PORCH_COMMAND=json.dumps(body))['id']
        cat=mutate(op='create_category',name='Rooms',position=0)
        for i,name in enumerate(['general','plans','random']):
            mutate(op='create_channel',name=name,kind='text',category_id=cat,position=i,synced=True)
        mutate(op='create_channel',name='hangout',kind='voice',category_id=cat,position=3,synced=True)
        invite=cli('invite-create',8,3600)['code']
        join=cli('community-invite-create',community,8,3600)['code']
        tok={'sam':sam['token']}; ids={}
        for name in ['maya','eli','jordan','taylor','alex']:
            auth=cli('register',PORCH_USERNAME=name,PORCH_PASSWORD=password,PORCH_INVITE=invite)
            tok[name]=auth['token']; ids[name]=auth['user']['id']
            cli('community-join',PORCH_TOKEN=auth['token'],PORCH_COMMUNITY_INVITE=join)
        heads=cli('conversations'); general=next(h for h in heads if h.get('community_id')==community and h['name']=='general')
        n=0
        for who,text in [('maya',"Here's the photo from the hike!"),('eli','Looks amazing.'),('jordan','Perfect. Let us plan a weekend trip?'),('sam','I can do the second weekend.')]:
            n+=1; send(general['id'],text,tok[who],n)
        notes=Path(data)/'notes.txt'; notes.write_text('Trip notes.\n'*4000)
        file=cli('file-upload',general['id'],notes,PORCH_TOKEN=tok['jordan'])
        n+=1; cli('message-send',general['id'],PORCH_TOKEN=tok['jordan'],PORCH_MESSAGE=json.dumps({'nonce':f'redesign_{n:020}','content':{'format':'markdown','text':'Notes from last time'},'attachments':[file['id']],'reply_to':None}))
        for name in ['maya','eli','jordan']:
            dm=cli('dm-create',PORCH_COMMAND=json.dumps({'participants':[ids[name]]}))
            for who,text in [(name,'Hey! Want to grab dinner this week?'),('sam','Definitely. Are you free tomorrow?'),(name,'Tomorrow works! How about 7 on Pine?'),('sam','That sounds great.')]:
                n+=1; send(dm['id'],text,sam['token'] if who=='sam' else tok[who],n)
        pins=['c:%d'%community,'u:%d'%ids['maya'],'u:%d'%ids['eli']]  # group tab first so click positions are deterministic
        ddir=Path(data)/'cache'; ddir.mkdir()
        (ddir/'settings.json').write_text(json.dumps({'font_size':15,'members':False,'origin':env['PORCH_URL'],'pins':pins}))
        disp=f':{dnum}'
        xvfb=subprocess.Popen(['Xvfb',disp,'-screen','0','1924x1200x24'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL); time.sleep(1)
        ui={**env,'DISPLAY':disp,'PORCH_DATA_DIR':str(ddir),'PORCH_SMOKE_SECONDS':'150','VK_ICD_FILENAMES':'/usr/share/vulkan/icd.d/lvp_icd.json'}
        wm=subprocess.Popen(['openbox'],env=ui,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        desktop=subprocess.Popen([bin/'porch-desktop'],env=ui,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        wid=subprocess.run(['xdotool','search','--sync','--name','^Porch$'],env=ui,timeout=30,capture_output=True,text=True).stdout.split()[0]
        subprocess.run(['xdotool','windowmove',wid,'0','0','windowsize',wid,'1920','1080'],env=ui)
        time.sleep(3)
        print(subprocess.run(['xdotool','getwindowgeometry',wid],env=ui,capture_output=True,text=True).stdout)
        def click(x,y): subprocess.run(['xdotool','mousemove',str(x),str(y),'click','1'],env=ui,check=True)
        def shot(name):
            out=root/'docs/evidence'; out.mkdir(exist_ok=True)
            subprocess.run(['import','-window',wid,str(out/f'redesign-{name}.png')],env=ui,check=True)
        cx,cy=960,540
        # login form is centered; fields sit at fixed offsets from center
        for dy,value in [(-74,env['PORCH_URL']),(-21,'sam'),(31,password)]:
            click(cx,cy+dy)
            subprocess.run(['xdotool','key','ctrl+a'],env=ui)
            subprocess.run(['xdotool','type','--clearmodifiers','--file','-'],input=value,text=True,env=ui,check=True)
        click(cx,cy+82); time.sleep(6)
        # window sits at (2,40) on screen; coordinates below are window-relative
        def wclick(x,y): click(x+2,y+40)
        wclick(45,28); time.sleep(3); shot('home')
        wclick(int(os.environ.get('TAB_GROUP_X','150')),28); time.sleep(2)
        wclick(130,int(os.environ.get('ROOM_Y','110'))); time.sleep(3); shot('group')
        wclick(int(os.environ.get('TAB_MAYA_X','252')),28); time.sleep(3); shot('dm')
        print('captured; inspect before claiming')
    finally:
        for p in (desktop,wm,xvfb,server):
            if p and p.poll() is None:
                p.terminate()
                try: p.wait(timeout=5)
                except subprocess.TimeoutExpired: p.kill(); p.wait()
