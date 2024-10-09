
using StarBreaker.Common;

namespace StarBreaker.Chf;

public sealed class StarCitizenCharacter
{
    public required BodyTypeProperty BodyType { get; init; }
    public required DnaProperty Dna { get; init; }
    public required BodyProperty Body { get; init; }
    public required HeadMaterialproperty HeadMaterial { get; init; }
    public required FaceMaterialProperty FaceMaterial { get; init; }
    public required List<DyeProperty> Dyes { get; init; }
    public required EyeMaterialProperty EyeMaterial { get; init; }
    public required BodyMaterialProperty BodyMaterial { get; init; }

    public static StarCitizenCharacter FromBytes(ReadOnlySpan<byte> data)
    {
        var reader = new SpanReader(data);

        reader.Expect<uint>(2);
        reader.Expect<uint>(7);

        var gender = BodyTypeProperty.Read(ref reader);
        var dnaProperty = DnaProperty.Read(ref reader, gender.Type);
        var totalCount = reader.Read<ulong>();
        var body = BodyProperty.Read(ref reader);
        var headMaterial = HeadMaterialproperty.Read(ref reader);
        var faceMaterial = FaceMaterialProperty.Read(ref reader, headMaterial.Material);

        var props = new List<DyeProperty>();
        while (DyeProperty.DyeKeys.Contains(reader.Peek<uint>()))
        {
            props.Add(DyeProperty.Read(ref reader));
        }

        //sometimes we don't have eye material.
        var eyeMaterial = EyeMaterialProperty.Read(ref reader);
        var bodyMaterialInfo = BodyMaterialProperty.Read(ref reader);

        if (reader.Remaining != 0)
            throw new Exception($"Unexpected data at the end of the file: {reader.Remaining} bytes");

        return new StarCitizenCharacter
        {
            BodyType = gender,
            Dna = dnaProperty,
            Body = body,
            HeadMaterial = headMaterial,
            FaceMaterial = faceMaterial,
            EyeMaterial = eyeMaterial,
            BodyMaterial = bodyMaterialInfo,
            Dyes = props
        };
    }
}