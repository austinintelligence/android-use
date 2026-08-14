package dev.codex.aubridge;

import android.content.Context;
import android.net.Credentials;
import android.net.LocalServerSocket;
import android.net.LocalSocket;
import android.util.Base64;
import org.json.JSONArray;
import org.json.JSONException;
import java.io.BufferedInputStream;
import java.io.BufferedOutputStream;
import java.io.DataInputStream;
import java.io.DataOutputStream;
import java.io.EOFException;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.security.SecureRandom;
import java.util.concurrent.ArrayBlockingQueue;
import java.util.concurrent.ThreadPoolExecutor;
import java.util.concurrent.TimeUnit;

final class Bridge implements AutoCloseable {
    static final int MAX_FRAME=1_048_576;
    private final Context context;
    private final Capture capture;
    private final Vm vm;
    private final byte[] token=new byte[32],nonce=new byte[32];
    private final ThreadPoolExecutor workers=new ThreadPoolExecutor(2,2,30,TimeUnit.SECONDS,new ArrayBlockingQueue<>(8),r->{Thread t=new Thread(r,"au-worker");t.setDaemon(true);return t;},new ThreadPoolExecutor.AbortPolicy());
    private volatile boolean open=true;
    private LocalServerSocket bootstrap,command;

    Bridge(Context context){this.context=context;new SecureRandom().nextBytes(token);new SecureRandom().nextBytes(nonce);capture=new Capture(context,null);vm=new Vm(context,null,capture);}
    void start(){new Thread(()->listen(true),"au-bootstrap").start();new Thread(()->listen(false),"au-command").start();}
    private void listen(boolean boot){try{LocalServerSocket server=new LocalServerSocket(boot?"aubridge-bootstrap-v3":"aubridge-v3");if(boot)bootstrap=server;else command=server;while(open){LocalSocket socket=server.accept();try{workers.execute(()->handle(socket,boot));}catch(RuntimeException e){try{socket.close();}catch(IOException ignored){}}}}catch(IOException ignored){}}
    private void handle(LocalSocket socket,boolean boot){try(LocalSocket s=socket){s.setSoTimeout(35_000);Credentials peer=s.getPeerCredentials();if(peer==null||peer.getUid()!=2000)return;DataInputStream in=new DataInputStream(new BufferedInputStream(s.getInputStream()));DataOutputStream out=new DataOutputStream(new BufferedOutputStream(s.getOutputStream()));if(boot){JSONArray q=read(in);if(q.length()!=2||q.optLong(0,-1)!=0||!"bootstrap".equals(q.optString(1)))return;write(out,new JSONArray().put(0).put(0).put(b64(token)).put(b64(nonce)));return;}JSONArray hello=read(in);if(!auth(hello)){write(out,new JSONArray().put(0).put(7));return;}write(out,new JSONArray().put(0).put(0));long seq=0;while(open){JSONArray q;try{q=read(in);}catch(EOFException e){return;}long got=q.optLong(0,-1);if(got!=seq+1){write(out,new JSONArray().put(got).put(8));return;}seq=got;write(out,dispatch(q));}}catch(Exception ignored){}}
    private boolean auth(JSONArray q){if(q.length()!=4||q.optLong(0,-1)!=0||!"hello".equals(q.optString(1)))return false;return MessageDigest.isEqual(token,unb64(q.optString(2)))&&MessageDigest.isEqual(nonce,unb64(q.optString(3)));}
    private JSONArray dispatch(JSONArray q){long seq=q.optLong(0);try{String name=q.getString(1);Ui ui=Ui.get();switch(name){case"status":return new JSONArray().put(seq).put(0).put(ui==null?0:ui.generation()).put(ui==null?0:7);case"observe":if(ui==null)return err(seq,10);return ui.observe(seq,q.isNull(2)?null:q.optString(2),q.optInt(3));case"capabilities":return new JSONArray().put(seq).put(0).put(Capture.capabilities(context));case"location":return new JSONArray().put(seq).put(0).put(Location.read(context));case"notifications":return new JSONArray().put(seq).put(0).put(Notify.read(context));case"run":return vm.run(seq,q);case"artifact":return capture.read(seq,q.optString(2),q.isNull(3)?null:q.optLong(3),q.isNull(4)?null:q.optLong(4));default:return err(seq,5);}}catch(Exception e){return err(seq,e instanceof Limit?6:e instanceof Vm.Permission?10:9);}}
    static JSONArray err(long seq,int code){return new JSONArray().put(seq).put(code);}
    static JSONArray read(DataInputStream in)throws IOException,JSONException{int n=in.readInt();if(n<=0||n>MAX_FRAME)throw new IOException("frame");byte[] b=new byte[n];in.readFully(b);return new JSONArray(new String(b,StandardCharsets.UTF_8));}
    static void write(DataOutputStream out,JSONArray value)throws IOException{byte[] b=value.toString().getBytes(StandardCharsets.UTF_8);if(b.length>MAX_FRAME)throw new IOException("frame");out.writeInt(b.length);out.write(b);out.flush();}
    private static String b64(byte[] b){return Base64.encodeToString(b,Base64.NO_WRAP|Base64.URL_SAFE);}
    private static byte[] unb64(String s){try{return Base64.decode(s,Base64.NO_WRAP|Base64.URL_SAFE);}catch(IllegalArgumentException e){return new byte[0];}}
    @Override public void close(){open=false;try{if(bootstrap!=null)bootstrap.close();}catch(IOException ignored){}try{if(command!=null)command.close();}catch(IOException ignored){}workers.shutdownNow();capture.close();}
    static final class Limit extends Exception { Limit(){super("limit");} }
}
