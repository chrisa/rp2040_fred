(cd firmware && rsync -vr src pi@192.168.3.87:rp2040_fred/rp2040_fred/firmware/)
(cd host && rsync -vr src pi@192.168.3.87:rp2040_fred/rp2040_fred/host/)
(cd protocol && rsync -vr src pi@192.168.3.87:rp2040_fred/rp2040_fred/protocol/)
(cd python && rsync -vr rust_ext/src pi@192.168.3.87:rp2040_fred/rp2040_fred/python/rust_ext/)
