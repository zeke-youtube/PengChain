// OpenCL 1.2: one work item searches one canonical big-endian u64 nonce.
// Round one hashes prefix || nonce; rounds 2–10 hash raw 32-byte digests.
__constant uint K[64] = {
    0x428a2f98,0x71374491,0xb5c0fbcf,0xe9b5dba5,0x3956c25b,0x59f111f1,0x923f82a4,0xab1c5ed5,
    0xd807aa98,0x12835b01,0x243185be,0x550c7dc3,0x72be5d74,0x80deb1fe,0x9bdc06a7,0xc19bf174,
    0xe49b69c1,0xefbe4786,0x0fc19dc6,0x240ca1cc,0x2de92c6f,0x4a7484aa,0x5cb0a9dc,0x76f988da,
    0x983e5152,0xa831c66d,0xb00327c8,0xbf597fc7,0xc6e00bf3,0xd5a79147,0x06ca6351,0x14292967,
    0x27b70a85,0x2e1b2138,0x4d2c6dfc,0x53380d13,0x650a7354,0x766a0abb,0x81c2c92e,0x92722c85,
    0xa2bfe8a1,0xa81a664b,0xc24b8b70,0xc76c51a3,0xd192e819,0xd6990624,0xf40e3585,0x106aa070,
    0x19a4c116,0x1e376c08,0x2748774c,0x34b0bcb5,0x391c0cb3,0x4ed8aa4a,0x5b9cca4f,0x682e6ff3,
    0x748f82ee,0x78a5636f,0x84c87814,0x8cc70208,0x90befffa,0xa4506ceb,0xbef9a3f7,0xc67178f2
};

uint rotr(uint x, uint n) { return (x >> n) | (x << (32 - n)); }

void init_state(uint h[8]) {
    h[0]=0x6a09e667; h[1]=0xbb67ae85; h[2]=0x3c6ef372; h[3]=0xa54ff53a;
    h[4]=0x510e527f; h[5]=0x9b05688c; h[6]=0x1f83d9ab; h[7]=0x5be0cd19;
}

void compress(uint h[8], uint w[64]) {
    for (uint i=16; i<64; i++) {
        uint a=w[i-15], b=w[i-2];
        uint s0=rotr(a,7)^rotr(a,18)^(a>>3);
        uint s1=rotr(b,17)^rotr(b,19)^(b>>10);
        w[i]=w[i-16]+s0+w[i-7]+s1;
    }
    uint a=h[0],b=h[1],c=h[2],d=h[3],e=h[4],f=h[5],g=h[6],z=h[7];
    for (uint i=0; i<64; i++) {
        uint s1=rotr(e,6)^rotr(e,11)^rotr(e,25);
        uint t1=z+s1+((e&f)^((~e)&g))+K[i]+w[i];
        uint s0=rotr(a,2)^rotr(a,13)^rotr(a,22);
        uint t2=s0+((a&b)^(a&c)^(b&c));
        z=g; g=f; f=e; e=d+t1; d=c; c=b; b=a; a=t1+t2;
    }
    h[0]+=a; h[1]+=b; h[2]+=c; h[3]+=d;
    h[4]+=e; h[5]+=f; h[6]+=g; h[7]+=z;
}

void hash_nonce(__global const uchar *prefix, uint prefix_len, ulong nonce, uint h[8]) {
    init_state(h);
    ulong length=(ulong)prefix_len+8;
    ulong padded=((length+9+63)/64)*64;
    ulong bits=length*8;
    for (ulong block=0; block<padded; block+=64) {
        uint w[64];
        for (uint i=0; i<16; i++) w[i]=0;
        for (uint i=0; i<64; i++) {
            ulong pos=block+i;
            uchar byte=0;
            if (pos<prefix_len) byte=prefix[pos];
            else if (pos<length) byte=(uchar)(nonce >> (56-8*(uint)(pos-prefix_len)));
            else if (pos==length) byte=0x80;
            else if (pos>=padded-8) byte=(uchar)(bits >> (8*(uint)(padded-1-pos)));
            w[i/4] |= ((uint)byte) << (24-8*(i%4));
        }
        compress(h,w);
    }
    for (uint round=1; round<10; round++) {
        uint w[64];
        for (uint i=0; i<8; i++) w[i]=h[i];
        w[8]=0x80000000;
        for (uint i=9; i<15; i++) w[i]=0;
        w[15]=256;
        init_state(h);
        compress(h,w);
    }
}

__kernel void search(__global const uchar *prefix, uint prefix_len,
                     ulong base, uint difficulty, __global uint *winner) {
    uint index=(uint)get_global_id(0);
    uint h[8];
    hash_nonce(prefix,prefix_len,base+(ulong)index,h);
    for (uint i=0; i<difficulty; i++) {
        if (((h[i/8] >> (28-4*(i%8))) & 15) != 0) return;
    }
    atomic_min((volatile __global uint *)winner,index);
}

// Used by explicit OpenCL conformance tests; no host hash substitutes this work.
__kernel void digests(__global const uchar *prefix, uint prefix_len,
                      ulong base, __global uint *output) {
    size_t index=get_global_id(0);
    uint h[8];
    hash_nonce(prefix,prefix_len,base+(ulong)index,h);
    for (uint i=0; i<8; i++) output[index*8+i]=h[i];
}
