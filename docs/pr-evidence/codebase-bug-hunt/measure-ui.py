import ctypes,ctypes.util,json,os,pathlib,subprocess,sys,time
import psutil
binary,label,output=sys.argv[1:]
args=[binary,'--demo','--page=appearance','--font-session-reset','--interactive','--width=1120','--height=760']
x=ctypes.CDLL(ctypes.util.find_library('X11'));xt=ctypes.CDLL(ctypes.util.find_library('Xtst'))
x.XOpenDisplay.restype=ctypes.c_void_p;x.XOpenDisplay.argtypes=[ctypes.c_char_p]
x.XFlush.argtypes=[ctypes.c_void_p];x.XCloseDisplay.argtypes=[ctypes.c_void_p]
xt.XTestFakeMotionEvent.argtypes=[ctypes.c_void_p,ctypes.c_int,ctypes.c_int,ctypes.c_int,ctypes.c_ulong]
xt.XTestFakeButtonEvent.argtypes=[ctypes.c_void_p,ctypes.c_uint,ctypes.c_int,ctypes.c_ulong]
d=x.XOpenDisplay(os.environ['DISPLAY'].encode());assert d
log_path=pathlib.Path(output).with_suffix('.log')
def require_running(process):
 status=process.poll()
 if status is not None:
  raise RuntimeError(f'Preview exited with status {status}; see {log_path}')
with open(log_path,'w') as log:
 p=subprocess.Popen(args,stdout=log,stderr=log)
 try:
  native=psutil.Process(p.pid);time.sleep(3)
  require_running(p)
  xt.XTestFakeMotionEvent(d,0,850,450,0)
  for _ in range(100):
   xt.XTestFakeButtonEvent(d,5,1,0);xt.XTestFakeButtonEvent(d,5,0,0)
  xt.XTestFakeMotionEvent(d,0,1279,959,0);x.XFlush(d);time.sleep(3)
  require_running(p)
  initial=native.cpu_times();started=time.monotonic();samples=[]
  for _ in range(15):
   time.sleep(1);require_running(p)
   samples.append({'elapsed_s':time.monotonic()-started,'rss_bytes':native.memory_info().rss})
  require_running(p)
  final=native.cpu_times();elapsed=time.monotonic()-started
  result={'revision':label,'command':args,'warmup_s':6,'interaction':'100 wheel-down events in settings body, pointer moved out','duration_s':elapsed,'interval_s':1,'process_cpu_one_core_percent':100*((final.user+final.system)-(initial.user+initial.system))/elapsed,'peak_rss_bytes':max(s['rss_bytes'] for s in samples),'settled_rss_bytes':samples[-1]['rss_bytes'],'children':[{'pid':child.pid,'rss_bytes':child.memory_info().rss} for child in native.children(recursive=True)],'executable_bytes':pathlib.Path(binary).stat().st_size,'samples':samples}
 finally:
  p.terminate()
  try:p.wait(timeout=5)
  except subprocess.TimeoutExpired:p.kill();p.wait()
  x.XCloseDisplay(d)
pathlib.Path(output).write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps({k:v for k,v in result.items() if k not in ['command','samples']}))
