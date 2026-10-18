#!/usr/bin/env python3
"""Install exact signed Focal LLVM20 packages into the disposable QA host."""
from pathlib import Path
import concurrent.futures,gzip,hashlib,json,os,subprocess,sys,time,urllib.request
lock=json.loads(Path(sys.argv[1]).read_text())
actual=subprocess.check_output(['dpkg','--print-architecture'],text=True).strip()
assert actual==lock['architecture'],(actual,lock['architecture'])
root=Path('/tmp/chda-qa-llvm20');root.mkdir(exist_ok=True)
def fetch(url,path,wanted=None):
    for attempt in range(3):
        try:
            data=urllib.request.urlopen(url,timeout=60).read()
            if wanted:assert hashlib.sha256(data).hexdigest()==wanted,url
            path.write_bytes(data);return data
        except Exception:
            if attempt==2:raise
            time.sleep(1)
key=fetch('https://apt.llvm.org/llvm-snapshot.gpg.key',root/'key.asc',lock['signingKeySha256'])
home=root/'gnupg';home.mkdir(mode=0o700,exist_ok=True)
shown=subprocess.check_output(['gpg','--batch','--homedir',str(home),'--with-colons','--show-keys',str(root/'key.asc')],text=True)
assert lock['signingKeyFingerprint'] in [line.split(':')[9] for line in shown.splitlines() if line.startswith('fpr:')]
subprocess.run(['gpg','--batch','--yes','--homedir',str(home),'--dearmor','--output',str(root/'key.gpg'),str(root/'key.asc')],check=True)
base=lock['origin']+'dists/'+lock['distribution']+'/'
release=fetch(base+'Release',root/'Release',lock['releaseSha256'])
fetch(base+'Release.gpg',root/'Release.gpg')
signature=subprocess.run(['gpgv','--homedir',str(home),'--keyring',str(root/'key.gpg'),str(root/'Release.gpg'),str(root/'Release')],text=True,stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
(root/'signature.txt').write_text(signature.stdout)
assert signature.returncode==0,signature.stdout
index_path='main/binary-'+actual+'/Packages.gz'
section=release.decode().split('SHA256:\n',1)[1].split('\nSHA512:',1)[0]
entries={line.split()[2]:line.split()[0] for line in section.splitlines() if len(line.split())==3}
assert entries[index_path]==lock['packagesIndexSha256']
index=fetch(base+index_path,root/'Packages.gz',lock['packagesIndexSha256'])
paragraphs={}
for paragraph in gzip.decompress(index).decode().split('\n\n'):
    fields={}
    for line in paragraph.splitlines():
        if ': ' in line and not line.startswith(' '):
            key,value=line.split(': ',1);fields[key]=value
    if 'Package' in fields:paragraphs[fields['Package']]=fields
for package in lock['packages']:
    fields=paragraphs[package['Package']]
    for key in ['Version','Architecture','Filename','SHA256']:
        assert fields[key]==package[key],(package['Package'],key)
def download(package):
    path=root/Path(package['Filename']).name
    fetch(lock['origin']+package['Filename'],path,package['SHA256'])
    return str(path)
with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
    files=list(pool.map(download,lock['packages']))
subprocess.run(['apt-get','install','-y','--no-install-recommends']+files,check=True)
abi=subprocess.check_output(['getconf','GNU_LIBC_VERSION'],text=True).strip()
assert abi=='glibc 2.31',abi
version=subprocess.check_output(['/usr/lib/llvm-20/bin/llvm-config','--version'],text=True).strip()
assert version=='20.1.8',version
installed=subprocess.check_output(['dpkg-query','-W','-f=${binary:Package} ${Version}\n','libc6','libc-bin','libstdc++6']+[x['Package'] for x in lock['packages']],text=True)
(root/'installed.txt').write_text(installed)
(root/'provenance.json').write_text(json.dumps({'architecture':actual,'glibcAbi':abi,'llvm':version,'lock':lock,'signature':signature.stdout,'installed':installed},indent=2)+'\n')
