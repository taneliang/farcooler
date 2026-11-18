package main

/*
#cgo LDFLAGS: -llog
#include <android/log.h>
#include <stdlib.h>
#include <ifaddrs.h>
#include <net/if.h>
#include <netinet/in.h>
*/
import "C"

import (
	"fmt"
	"log"
	"net"
	"sort"
	"unsafe"

	"tailscale.com/net/netmon"
)

// Native stderr is not logcat. Keep the tunnel's diagnostics visible on Android.
type androidLog struct{}

func (androidLog) Write(p []byte) (int, error) {
	message := C.CString(string(p))
	tag := C.CString("Tailcat")
	defer C.free(unsafe.Pointer(message))
	defer C.free(unsafe.Pointer(tag))
	C.__android_log_write(C.ANDROID_LOG_INFO, tag, message)
	return len(p), nil
}

func init() {
	log.SetOutput(androidLog{})
	netmon.RegisterInterfaceGetter(androidInterfaces)
}

// Android denies Go's NETLINK_ROUTE bind for apps targeting SDK 30+.
// Bionic's getifaddrs is the platform-supported enumeration (also used by
// java.net.NetworkInterface). Supply addresses too: Interface.Addrs would
// otherwise go straight back to the forbidden Go netlink implementation.
func androidInterfaces() ([]netmon.Interface, error) {
	var head *C.struct_ifaddrs
	if rc, err := C.getifaddrs(&head); rc != 0 {
		return nil, fmt.Errorf("getifaddrs: %w", err)
	}
	defer C.freeifaddrs(head)
	byName := map[string]*netmon.Interface{}
	for row := head; row != nil; row = row.ifa_next {
		name := C.GoString(row.ifa_name)
		iface := byName[name]
		if iface == nil {
			var flags net.Flags
			for _, flag := range []struct {
				native C.uint
				goFlag net.Flags
			}{
				{C.IFF_UP, net.FlagUp}, {C.IFF_RUNNING, net.FlagRunning},
				{C.IFF_LOOPBACK, net.FlagLoopback}, {C.IFF_BROADCAST, net.FlagBroadcast},
				{C.IFF_POINTOPOINT, net.FlagPointToPoint}, {C.IFF_MULTICAST, net.FlagMulticast},
			} {
				if row.ifa_flags&flag.native != 0 {
					flags |= flag.goFlag
				}
			}
			iface = &netmon.Interface{Interface: &net.Interface{
				Name: name, Index: int(C.if_nametoindex(row.ifa_name)), Flags: flags,
			}, AltAddrs: []net.Addr{}}
			byName[name] = iface
		}
		ip, mask := androidIP(row.ifa_addr), androidIP(row.ifa_netmask)
		if ip != nil && mask != nil {
			iface.AltAddrs = append(iface.AltAddrs, &net.IPNet{IP: ip, Mask: net.IPMask(mask)})
		}
	}
	names := make([]string, 0, len(byName))
	for name := range byName {
		names = append(names, name)
	}
	sort.Strings(names)
	interfaces := make([]netmon.Interface, 0, len(names))
	for _, name := range names {
		interfaces = append(interfaces, *byName[name])
	}
	return interfaces, nil
}

func androidIP(addr *C.struct_sockaddr) net.IP {
	if addr == nil {
		return nil
	}
	switch addr.sa_family {
	case C.AF_INET:
		v4 := (*C.struct_sockaddr_in)(unsafe.Pointer(addr))
		return net.IP(C.GoBytes(unsafe.Pointer(&v4.sin_addr), 4))
	case C.AF_INET6:
		v6 := (*C.struct_sockaddr_in6)(unsafe.Pointer(addr))
		return net.IP(C.GoBytes(unsafe.Pointer(&v6.sin6_addr), 16))
	default:
		return nil
	}
}
