import socket, os, struct, time, sys, statistics
def bind(sock, host, port=3478):
    tid=os.urandom(12); req=struct.pack('!HHI',1,0,0x2112A442)+tid
    addr=(socket.gethostbyname(host),port); t=time.perf_counter(); sock.sendto(req,addr)
    while True:
        data,_=sock.recvfrom(2048)
        if data[8:20]==tid: break
    rtt=(time.perf_counter()-t)*1000; i=20; mapped=None
    while i<len(data):
        at,al=struct.unpack('!HH',data[i:i+4]); v=data[i+4:i+4+al]
        if at==0x20:
            p=struct.unpack('!H',v[2:4])[0]^0x2112; ip=bytes(a^b for a,b in zip(v[4:8],struct.pack('!I',0x2112A442)))
            mapped=(socket.inet_ntoa(ip),p)
        i+=4+al+((4-al%4)%4)
    return rtt,mapped
s=socket.socket(socket.AF_INET,socket.SOCK_DGRAM); s.settimeout(2); s.bind(('0.0.0.0',0))
for h in sys.argv[1:]:
    rtts=[];m=None
    for _ in range(8):
        try: r,m=bind(s,h); rtts.append(r)
        except Exception as e: pass
    print(f"{h:24} replies={len(rtts)}/8 median_rtt={statistics.median(rtts) if rtts else float('nan'):.1f}ms mapped_port={m[1] if m else None}")
