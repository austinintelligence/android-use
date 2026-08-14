package dev.codex.aubridge;

import android.accessibilityservice.AccessibilityService;
import android.Manifest;
import android.content.Context;
import android.content.pm.PackageManager;
import android.graphics.Bitmap;
import android.graphics.ColorSpace;
import android.graphics.ImageFormat;
import android.hardware.HardwareBuffer;
import android.hardware.camera2.CameraCaptureSession;
import android.hardware.camera2.CameraCharacteristics;
import android.hardware.camera2.CameraDevice;
import android.hardware.camera2.CameraManager;
import android.hardware.camera2.CaptureRequest;
import android.hardware.camera2.params.StreamConfigurationMap;
import android.media.AudioFormat;
import android.media.AudioRecord;
import android.media.MediaRecorder;
import android.media.projection.MediaProjection;
import android.os.Build;
import android.os.Handler;
import android.os.HandlerThread;
import android.os.SystemClock;
import android.util.Base64;
import android.util.DisplayMetrics;
import android.util.Size;
import android.hardware.display.VirtualDisplay;
import android.view.Display;
import android.view.Surface;
import android.view.WindowManager;
import org.json.JSONArray;
import org.json.JSONObject;
import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.RandomAccessFile;
import java.nio.ByteBuffer;
import java.util.ArrayList;
import java.nio.file.Files;
import java.util.Arrays;
import java.util.Comparator;
import java.util.List;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicLong;

final class Capture implements AutoCloseable {
    private static final long MAX_ARTIFACT=16L*1024L*1024L;
    private final Context context;private final File dir;private final AtomicLong next=new AtomicLong(System.currentTimeMillis());
    Capture(Context context,Ui ignored){this.context=context;dir=new File(context.getNoBackupFilesDir(),"artifacts");dir.mkdirs();prune();}
    static JSONObject capabilities(Context context)throws Exception{boolean camera=false;try{camera=context.getSystemService(CameraManager.class).getCameraIdList().length>0;}catch(Exception ignored){}return new JSONObject().put("camera",camera).put("camera_permission",context.checkSelfPermission(Manifest.permission.CAMERA)==PackageManager.PERMISSION_GRANTED).put("microphone",context.getSystemService(Context.AUDIO_SERVICE)!=null).put("microphone_permission",context.checkSelfPermission(Manifest.permission.RECORD_AUDIO)==PackageManager.PERMISSION_GRANTED).put("microphone_rate",16000).put("microphone_channels",1).put("screen_capture",Build.VERSION.SDK_INT>=30&&Ui.get()!=null).put("screen_record",Build.VERSION.SDK_INT>=21&&Projection.available()).put("screen_record_format","mp4").put("location_permission",context.checkSelfPermission(Manifest.permission.ACCESS_COARSE_LOCATION)==PackageManager.PERMISSION_GRANTED||context.checkSelfPermission(Manifest.permission.ACCESS_FINE_LOCATION)==PackageManager.PERMISSION_GRANTED).put("notifications",Notify.read(context).optBoolean("enabled"));}
    String screen()throws Exception{if(Build.VERSION.SDK_INT<30)throw new Vm.Unsupported();Ui ui=Ui.get();if(ui==null)throw new Vm.Unsupported();String id="a"+Long.toUnsignedString(next.incrementAndGet(),36);File file=file(id);CountDownLatch done=new CountDownLatch(1);Throwable[] failure=new Throwable[1];ui.takeScreenshot(Display.DEFAULT_DISPLAY,context.getMainExecutor(),new AccessibilityService.TakeScreenshotCallback(){@Override public void onSuccess(AccessibilityService.ScreenshotResult result){try(HardwareBuffer buffer=result.getHardwareBuffer()){ColorSpace color=result.getColorSpace();Bitmap hardware=Bitmap.wrapHardwareBuffer(buffer,color);if(hardware==null)throw new IOException("bitmap");Bitmap bitmap=hardware.copy(Bitmap.Config.ARGB_8888,false);try(FileOutputStream out=new FileOutputStream(file)){if(bitmap==null||!bitmap.compress(Bitmap.CompressFormat.PNG,100,out))throw new IOException("png");out.getFD().sync();}if(bitmap!=null)bitmap.recycle();hardware.recycle();}catch(Throwable e){failure[0]=e;}finally{done.countDown();}}@Override public void onFailure(int errorCode){failure[0]=new IOException("capture");done.countDown();}});if(!done.await(10,TimeUnit.SECONDS)||failure[0]!=null||file.length()>MAX_ARTIFACT){file.delete();return null;}prune();return id;}
    String camera(String request)throws Exception {
        String facing=request;int requestedW=0,requestedH=0;int separator=request.indexOf('|');
        if(separator>=0){facing=request.substring(0,separator);String dimensions=request.substring(separator+1);int x=dimensions.indexOf('x');if(x<=0)throw new Bridge.Limit();try{requestedW=Integer.parseInt(dimensions.substring(0,x));requestedH=Integer.parseInt(dimensions.substring(x+1));}catch(NumberFormatException e){throw new Bridge.Limit();}if(requestedW<160||requestedW>4096||requestedH<160||requestedH>4096)throw new Bridge.Limit();}
        if(context.checkSelfPermission(Manifest.permission.CAMERA)!=PackageManager.PERMISSION_GRANTED)throw new Vm.Permission();CameraManager manager=context.getSystemService(CameraManager.class);String camera=chooseCamera(manager,facing);CameraCharacteristics characteristics=manager.getCameraCharacteristics(camera);Size size=chooseSize(characteristics.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP),requestedW,requestedH);String id="a"+Long.toUnsignedString(next.incrementAndGet(),36);File output=file(id);HandlerThread thread=new HandlerThread("au-camera");thread.start();Handler handler=new Handler(thread.getLooper());android.media.ImageReader reader=android.media.ImageReader.newInstance(size.getWidth(),size.getHeight(),ImageFormat.JPEG,2);CountDownLatch done=new CountDownLatch(1);Throwable[] failure=new Throwable[1];CameraDevice[] device=new CameraDevice[1];CameraCaptureSession[] session=new CameraCaptureSession[1];reader.setOnImageAvailableListener(source->{try(android.media.Image image=source.acquireLatestImage();FileOutputStream stream=new FileOutputStream(output)){if(image==null)throw new IOException("image");ByteBuffer buffer=image.getPlanes()[0].getBuffer();byte[] bytes=new byte[buffer.remaining()];buffer.get(bytes);stream.write(bytes);stream.getFD().sync();}catch(Throwable error){failure[0]=error;}finally{done.countDown();}},handler);try{manager.openCamera(camera,new CameraDevice.StateCallback(){@Override public void onOpened(CameraDevice opened){device[0]=opened;try{List<Surface> surfaces=new ArrayList<>();surfaces.add(reader.getSurface());opened.createCaptureSession(surfaces,new CameraCaptureSession.StateCallback(){@Override public void onConfigured(CameraCaptureSession configured){session[0]=configured;try{CaptureRequest.Builder builder=opened.createCaptureRequest(CameraDevice.TEMPLATE_STILL_CAPTURE);builder.addTarget(reader.getSurface());Integer sensor=characteristics.get(CameraCharacteristics.SENSOR_ORIENTATION);WindowManager windows=context.getSystemService(WindowManager.class);Display display=windows==null?null:windows.getDefaultDisplay();int rotation=display==null?Surface.ROTATION_0:display.getRotation();int degrees=rotation==Surface.ROTATION_90?90:rotation==Surface.ROTATION_180?180:rotation==Surface.ROTATION_270?270:0;boolean front=characteristics.get(CameraCharacteristics.LENS_FACING)!=null&&characteristics.get(CameraCharacteristics.LENS_FACING)==CameraCharacteristics.LENS_FACING_FRONT;int orientation=(sensor==null?0:sensor)+(front?degrees:-degrees);builder.set(CaptureRequest.JPEG_ORIENTATION,(orientation+360)%360);configured.capture(builder.build(),null,handler);}catch(Exception error){failure[0]=error;done.countDown();}}@Override public void onConfigureFailed(CameraCaptureSession ignored){failure[0]=new IOException("camera session");done.countDown();}},handler);}catch(Exception error){failure[0]=error;done.countDown();}}@Override public void onDisconnected(CameraDevice opened){opened.close();failure[0]=new IOException("camera disconnected");done.countDown();}@Override public void onError(CameraDevice opened,int error){opened.close();failure[0]=new IOException("camera error");done.countDown();}},handler);if(!done.await(8,TimeUnit.SECONDS))throw new Vm.Timeout();if(failure[0]!=null)throw new IOException(failure[0]);if(!output.isFile()||output.length()==0||output.length()>MAX_ARTIFACT)throw new IOException("camera produced no JPEG");return id;}finally{if(session[0]!=null)session[0].close();if(device[0]!=null)device[0].close();reader.close();thread.quitSafely();if(!output.isFile()||output.length()==0||output.length()>MAX_ARTIFACT)output.delete();prune();}}
    String microphone(int seconds)throws Exception{if(context.checkSelfPermission(Manifest.permission.RECORD_AUDIO)!=PackageManager.PERMISSION_GRANTED)throw new Vm.Permission();int rate=16000,mask=AudioFormat.CHANNEL_IN_MONO,min=AudioRecord.getMinBufferSize(rate,mask,AudioFormat.ENCODING_PCM_16BIT);if(min<=0)throw new Vm.Unsupported();AudioRecord record=new AudioRecord.Builder().setAudioSource(MediaRecorder.AudioSource.MIC).setAudioFormat(new AudioFormat.Builder().setEncoding(AudioFormat.ENCODING_PCM_16BIT).setSampleRate(rate).setChannelMask(mask).build()).setBufferSizeInBytes(min*2).build();String id="a"+Long.toUnsignedString(next.incrementAndGet(),36);File output=file(id);byte[] buffer=new byte[min];long data=0;boolean ok=false;try(RandomAccessFile out=new RandomAccessFile(output,"rw")){out.write(new byte[44]);record.startRecording();long end=System.currentTimeMillis()+seconds*1000L;while(System.currentTimeMillis()<end){int n=record.read(buffer,0,buffer.length,AudioRecord.READ_NON_BLOCKING);if(n>0){out.write(buffer,0,n);data+=n;}else{SystemClock.sleep(10);}}if(data==0)throw new IOException("microphone produced no samples");out.seek(0);wav(out,data,rate);ok=true;}finally{if(record.getRecordingState()==AudioRecord.RECORDSTATE_RECORDING)record.stop();record.release();if(!ok||output.length()>MAX_ARTIFACT)output.delete();prune();}return id;}
    String screenRecord(int seconds)throws Exception{
        if(Build.VERSION.SDK_INT<21)throw new Vm.Unsupported();
        if(!Projection.available())throw new Vm.Permission();
        MediaProjection projection;
        try{projection=Projection.acquire(context);}catch(SecurityException e){throw new Vm.Permission();}
        if(projection==null)throw new Vm.Permission();
        DisplayMetrics metrics=context.getResources().getDisplayMetrics();
        int width=Math.max(320,metrics.widthPixels),height=Math.max(320,metrics.heightPixels),max=Math.max(width,height);
        if(max>1920){double scale=1920.0/max;width=(int)Math.round(width*scale);height=(int)Math.round(height*scale);}
        width=Math.max(2,width&~1);height=Math.max(2,height&~1);
        String id="a"+Long.toUnsignedString(next.incrementAndGet(),36);File output=file(id);MediaRecorder recorder=new MediaRecorder();VirtualDisplay display=null;boolean started=false,ok=false;
        try{
            BridgeService.projectionStarted();
            recorder.setVideoSource(MediaRecorder.VideoSource.SURFACE);recorder.setOutputFormat(MediaRecorder.OutputFormat.MPEG_4);recorder.setVideoEncoder(MediaRecorder.VideoEncoder.H264);recorder.setVideoSize(width,height);recorder.setVideoFrameRate(20);recorder.setVideoEncodingBitRate(Math.min(4_000_000,Math.max(800_000,width*height*2)));recorder.setOutputFile(output.getAbsolutePath());recorder.prepare();
            display=projection.createVirtualDisplay("AU screen",width,height,metrics.densityDpi,android.hardware.display.DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR,recorder.getSurface(),null,null);if(display==null)throw new IOException("virtual display");recorder.start();started=true;SystemClock.sleep(seconds*1000L);recorder.stop();if(output.length()>MAX_ARTIFACT)throw new IOException("recording too large");ok=true;return id;
        }catch(RuntimeException e){if(started)try{recorder.stop();}catch(RuntimeException ignored){}throw e;
        }finally{if(display!=null)display.release();try{recorder.reset();}catch(RuntimeException ignored){}recorder.release();projection.stop();if(!ok||output.length()>MAX_ARTIFACT)output.delete();prune();}
    }
    JSONArray read(long seq,String id,Long from,Long to)throws Exception{if(!id.matches("a[0-9a-z]{1,32}"))throw new Bridge.Limit();File f=file(id);if(!f.isFile()||Files.isSymbolicLink(f.toPath()))throw new IOException("artifact");long size=f.length();if(size>MAX_ARTIFACT)throw new Bridge.Limit();long start=from==null?0:from,end=to==null?Math.min(size,start+2800):to;if(start<0||end<start||end>size||end-start>2800)throw new Bridge.Limit();byte[] bytes=new byte[(int)(end-start)];try(RandomAccessFile in=new RandomAccessFile(f,"r")){in.seek(start);in.readFully(bytes);}return new JSONArray().put(seq).put(0).put(size).put(start).put(Base64.encodeToString(bytes,Base64.NO_WRAP));}
    private File file(String id)throws IOException{File f=new File(dir,id);if(!f.getCanonicalFile().getParentFile().equals(dir.getCanonicalFile()))throw new IOException("artifact");return f;}
    private static String chooseCamera(CameraManager manager,String facing)throws Exception{for(String id:manager.getCameraIdList()){Integer lens=manager.getCameraCharacteristics(id).get(CameraCharacteristics.LENS_FACING);if((facing.isEmpty()||"rear".equals(facing))&&lens!=null&&lens==CameraCharacteristics.LENS_FACING_BACK)return id;if("front".equals(facing)&&lens!=null&&lens==CameraCharacteristics.LENS_FACING_FRONT)return id;}String[] ids=manager.getCameraIdList();if(ids.length==0)throw new Vm.Unsupported();return ids[0];}
    private static Size chooseSize(StreamConfigurationMap map)throws Exception{return chooseSize(map,0,0);}
    private static Size chooseSize(StreamConfigurationMap map,int requestedW,int requestedH)throws Exception{if(map==null||map.getOutputSizes(ImageFormat.JPEG)==null)throw new Vm.Unsupported();Size[] sizes=map.getOutputSizes(ImageFormat.JPEG);if(requestedW>0&&requestedH>0){Size selected=sizes[0];long score=Long.MAX_VALUE;for(Size size:sizes){long candidate=Math.abs((long)size.getWidth()-requestedW)+Math.abs((long)size.getHeight()-requestedH);if(candidate<score){selected=size;score=candidate;}}return selected;}Size selected=sizes[0];long area=0;for(Size size:sizes){long next=(long)size.getWidth()*size.getHeight();if(size.getWidth()<=1280&&size.getHeight()<=1280&&next>area){selected=size;area=next;}}return selected;}
    private static void wav(RandomAccessFile out,long data,int rate)throws Exception{out.writeBytes("RIFF");le(out,(int)(36+data));out.writeBytes("WAVEfmt ");le(out,16);les(out,(short)1);les(out,(short)1);le(out,rate);le(out,rate*2);les(out,(short)2);les(out,(short)16);out.writeBytes("data");le(out,(int)data);}
    private static void le(RandomAccessFile out,int v)throws Exception{out.write(v&255);out.write((v>>>8)&255);out.write((v>>>16)&255);out.write((v>>>24)&255);}private static void les(RandomAccessFile out,short v)throws Exception{le(out,v&65535);}
    private void prune(){File[] files=dir.listFiles(File::isFile);if(files==null||files.length<=16)return;Arrays.sort(files,Comparator.comparingLong(File::lastModified));for(int i=0;i<files.length-16;i++)files[i].delete();}
    @Override public void close(){}
}
